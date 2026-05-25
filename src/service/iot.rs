use crate::UndocApiArguments;
use crate::service::state::StateHandle;
use anyhow::Context;
use govee_api::ble::{Base64HexBytes, GoveeBlePacket, HumidifierAutoMode, NotifyHumidifierMode};
use govee_api::lan_api::{DeviceColor, DeviceStatus};
use govee_api::platform_api::from_json;
use govee_api::undoc_api::{
    DeviceEntry, GoveeUndocumentedApi, LoginAccountResponse, ParsedOneClick, ms_timestamp,
};
use rumqttc::{
    AsyncClient, Event, EventLoop, MqttOptions, Packet as MqttPacket, QoS, TlsConfiguration,
    Transport,
};
use serde::Deserialize;
use serde_json::{Map, Value as JsonValue, json};
use std::time::Duration;

/// Govee IoT state packets carry effect/scene blobs that can exceed rumqttc's
/// 10 KiB default incoming packet limit, so raise the ceiling.
const MAX_PACKET_SIZE: usize = 256 * 1024;

#[derive(Clone)]
pub struct IotClient {
    client: AsyncClient,
    /// The account topic (GA/...), included as `msg.accountTopic` on every
    /// message. The Govee app sends this on all writes (see AbsCmdWrite in the
    /// app); some devices ignore messages that omit it.
    account_topic: String,
    /// Used to relay messages for shared devices via the REST API; see
    /// [`IotClient::send_msg`].
    undoc: GoveeUndocumentedApi,
}

impl IotClient {
    pub fn is_device_compatible(&self, device: &DeviceEntry) -> bool {
        device.device_ext.device_settings.topic.is_some()
    }

    /// Send an IoT message (cmd + optional data) to a device, choosing the
    /// transport the Govee app uses for it: shared devices go through the REST
    /// relay (which carries the `gas` authorization), owned devices publish
    /// MQTT directly to the device topic. `cmd_type` is 0 for reads (status)
    /// and 1 for writes (control).
    async fn send_msg(
        &self,
        device: &DeviceEntry,
        cmd: &str,
        data: Option<JsonValue>,
        cmd_type: u8,
        cmd_version: u8,
    ) -> anyhow::Result<()> {
        let mut msg = Map::new();
        msg.insert("cmd".into(), json!(cmd));
        if let Some(data) = data {
            msg.insert("data".into(), data);
        }
        msg.insert("cmdVersion".into(), json!(cmd_version));
        msg.insert("type".into(), json!(cmd_type));

        if device.is_shared() {
            // The device ignores direct MQTT publishes from a guest account;
            // relay through Govee's REST API, which carries the gas token.
            return self.undoc.control_device(device, msg).await;
        }

        let device_topic = device.device_topic()?;
        msg.insert(
            "transaction".into(),
            json!(format!("v_{}000", ms_timestamp())),
        );
        msg.insert("accountTopic".into(), json!(self.account_topic));
        self.client
            .publish(
                device_topic,
                QoS::AtMostOnce,
                false,
                serde_json::to_string(&json!({ "msg": msg }))?,
            )
            .await
            .with_context(|| format!("IotClient::send_msg {cmd} for {}", device.device))?;
        Ok(())
    }

    pub async fn request_status_update(&self, device: &DeviceEntry) -> anyhow::Result<()> {
        // cmdVersion 0 matches the Govee app (AbsCmd default) for status across
        // the device fleet; only a few legacy SKUs override it.
        self.send_msg(device, "status", None, 0, 0).await
    }

    pub async fn set_power_state(&self, device: &DeviceEntry, on: bool) -> anyhow::Result<()> {
        log::trace!("set_power_state for {} to {on}", device.device);
        self.send_msg(device, "turn", Some(json!({ "val": on as u8 })), 1, 0)
            .await
    }

    /// Set the power state of a Wi-Fi smart plug/switch. `outlet` is the
    /// zero-based outlet index, or 15 to address all outlets (single-outlet
    /// plugs). The packed `val` matches the Govee app's CmdTurn.getCmd.
    pub async fn set_socket_power(
        &self,
        device: &DeviceEntry,
        outlet: u8,
        on: bool,
    ) -> anyhow::Result<()> {
        let val = socket_turn_val(outlet, on);
        log::trace!(
            "set_socket_power for {} outlet={outlet} on={on} val={val}",
            device.device
        );
        self.send_msg(device, "turn", Some(json!({ "val": val })), 1, 0)
            .await
    }

    pub async fn set_brightness(&self, device: &DeviceEntry, percent: u8) -> anyhow::Result<()> {
        log::trace!("set_brightness for {} to {percent}", device.device);
        self.send_msg(device, "brightness", Some(json!({ "val": percent })), 1, 0)
            .await
    }

    pub async fn set_color_temperature(
        &self,
        device: &DeviceEntry,
        kelvin: u32,
    ) -> anyhow::Result<()> {
        log::trace!("set_color_temperature for {} to {kelvin}", device.device);
        let data = json!({
            "color": { "r": 0, "g": 0, "b": 0 },
            "colorTemInKelvin": kelvin,
        });
        self.send_msg(device, "colorwc", Some(data), 1, 0).await
    }

    pub async fn set_color_rgb(
        &self,
        device: &DeviceEntry,
        r: u8,
        g: u8,
        b: u8,
    ) -> anyhow::Result<()> {
        log::trace!("set_color_rgb for {} to {r},{g},{b}", device.device);
        let data = json!({
            "color": { "r": r, "g": g, "b": b },
            "colorTemInKelvin": 0,
        });
        self.send_msg(device, "colorwc", Some(data), 1, 0).await
    }

    pub async fn send_real(
        &self,
        device: &DeviceEntry,
        commands: Vec<String>,
    ) -> anyhow::Result<()> {
        log::trace!("send_real for {} to {commands:?}", device.device);
        self.send_msg(device, "ptReal", Some(json!({ "command": commands })), 1, 0)
            .await
    }

    pub async fn activate_one_click(&self, item: &ParsedOneClick) -> anyhow::Result<()> {
        for entry in &item.entries {
            for command in &entry.msgs {
                self.client
                    .publish(
                        entry.topic.as_str(),
                        QoS::AtMostOnce,
                        false,
                        serde_json::to_string(command)?,
                    )
                    .await
                    .context("sending OneClick")?;
            }
        }
        Ok(())
    }
}

pub async fn start_iot_client(
    args: &UndocApiArguments,
    state: StateHandle,
    acct: Option<LoginAccountResponse>,
) -> anyhow::Result<()> {
    let undoc_api = args.api_client()?;
    let acct = match acct {
        Some(a) => a,
        None => undoc_api.login_account_cached().await?,
    };
    log::trace!("{acct:#?}");
    let res = undoc_api.get_iot_key(&acct.token).await?;
    log::trace!("{res:#?}");

    let key_bytes = data_encoding::BASE64.decode(res.p12.as_bytes())?;

    // The PFX from Govee holds the per-account client certificate and private
    // key for mutual TLS to AWS IoT. rumqttc takes PEM bytes directly, so we
    // convert in memory rather than writing the key to disk.
    log::trace!("parsing IoT PFX key");
    let container = p12::PFX::parse(&key_bytes).context("PFX::parse")?;
    let mut key_pem = None;
    for key in container.key_bags(&res.p12_pass).context("key_bags")? {
        let priv_key = openssl::pkey::PKey::private_key_from_der(&key).context("from_der")?;
        key_pem = Some(
            priv_key
                .private_key_to_pem_pkcs8()
                .context("to_pem_pkcs8")?,
        );
    }
    let mut cert_pem = None;
    for cert in container.cert_bags(&res.p12_pass).context("cert_bags")? {
        let cert = openssl::x509::X509::from_der(&cert).context("x509 from der")?;
        cert_pem = Some(cert.to_pem().context("cert.to_pem")?);
    }
    let key_pem = key_pem.context("PFX contained no private key")?;
    let cert_pem = cert_pem.context("PFX contained no certificate")?;

    // Server verification uses the system CA bundle (the trust anchor that the
    // AWS IoT endpoint cert chains to). rumqttc's Simple config reads it into a
    // fresh root store; --amazon-root-ca points at the system bundle by default.
    let ca_pem = std::fs::read(&args.amazon_root_ca)
        .with_context(|| format!("reading CA bundle {}", args.amazon_root_ca.display()))?;

    let mut mqtt_options = MqttOptions::new(
        format!(
            "AP/{account_id}/{id}",
            account_id = *acct.account_id,
            id = uuid::Uuid::new_v4().simple()
        ),
        res.endpoint.clone(),
        8883,
    );
    mqtt_options.set_keep_alive(Duration::from_secs(120));
    mqtt_options.set_max_packet_size(MAX_PACKET_SIZE, MAX_PACKET_SIZE);
    mqtt_options.set_transport(Transport::Tls(TlsConfiguration::Simple {
        ca: ca_pem,
        alpn: None,
        client_auth: Some((cert_pem, key_pem)),
    }));

    let (client, eventloop) = AsyncClient::new(mqtt_options, 32);

    state
        .set_iot_client(IotClient {
            client: client.clone(),
            account_topic: acct.topic.to_string(),
            undoc: undoc_api,
        })
        .await;

    log::trace!("Connecting to IoT {} port 8883", res.endpoint);
    tokio::spawn(async move {
        if let Err(err) = run_iot_subscriber(eventloop, state, client, acct).await {
            log::error!("IoT loop failed: {err:#}");
        }
        log::info!("IoT loop terminated");
    });

    Ok(())
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct Packet {
    sku: Option<String>,
    device: Option<String>,
    /// may actually be found in msg.cmd
    cmd: Option<String>,
    /// This is an embedded json string
    msg: Option<String>,
    state: StateUpdate,
    op: Option<OpData>,
}

#[derive(Deserialize, Debug)]
struct StateUpdate {
    #[serde(rename = "onOff")]
    pub on_off: Option<u8>,
    pub brightness: Option<u8>,
    pub color: Option<DeviceColor>,
    #[serde(rename = "colorTemInKelvin")]
    pub color_temperature_kelvin: Option<u32>,
    pub sku: Option<String>,
    pub device: Option<String>,
}

#[derive(Deserialize, Debug)]
#[allow(unused)]
struct OpData {
    #[serde(default)]
    command: Vec<Base64HexBytes>,

    // The next 4 fields are sourced from H6199
    // <https://github.com/wez/govee2mqtt/issues/36>
    #[serde(rename = "modeValue", default)]
    mode_value: Vec<Base64HexBytes>,
    #[serde(rename = "sleepValue", default)]
    sleep_value: Vec<Base64HexBytes>,
    #[serde(rename = "wakeupValue", default)]
    wakeup_value: Vec<Base64HexBytes>,
    #[serde(rename = "timerValue", default)]
    timer_value: Vec<Base64HexBytes>,
}

impl Packet {
    /// The sku can be in a couple of different places(!)
    fn sku(&self) -> Option<&str> {
        if let Some(sku) = self.sku.as_deref() {
            return Some(sku);
        }
        self.state.sku.as_deref()
    }
    fn device(&self) -> Option<&str> {
        if let Some(device) = self.device.as_deref() {
            return Some(device);
        }
        self.state.device.as_deref()
    }

    fn sku_and_device(&self) -> Option<(&str, &str)> {
        let sku = self.sku()?;
        let device = self.device()?;
        Some((sku, device))
    }
}

async fn run_iot_subscriber(
    mut eventloop: EventLoop,
    state: StateHandle,
    client: AsyncClient,
    acct: LoginAccountResponse,
) -> anyhow::Result<()> {
    loop {
        let event = match eventloop.poll().await {
            Ok(event) => event,
            Err(err) => {
                // rumqttc reconnects on the next poll; log and keep going.
                log::warn!("IoT disconnected: {err:#}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        match event {
            Event::Incoming(MqttPacket::Publish(msg)) => {
                let payload = String::from_utf8_lossy(&msg.payload);
                log::trace!("{} -> {payload}", msg.topic);

                match from_json::<Packet, _>(&msg.payload) {
                    Ok(packet) => {
                        log::debug!("{packet:?}");
                        if let Some((sku, device_id)) = packet.sku_and_device() {
                            {
                                let mut device = state.device_mut(sku, device_id).await;
                                let mut state = match device.iot_device_status.clone() {
                                    Some(state) => state,
                                    None => match device.device_state() {
                                        Some(state) => DeviceStatus {
                                            on: state.on,
                                            brightness: state.brightness,
                                            color: state.color,
                                            color_temperature_kelvin: state.kelvin,
                                        },
                                        None => DeviceStatus::default(),
                                    },
                                };

                                if let Some(v) = packet.state.brightness {
                                    state.brightness = v;
                                    state.on = v != 0;
                                }
                                if let Some(v) = packet.state.color {
                                    state.color = v;
                                    state.on = true;
                                }
                                if let Some(v) = packet.state.color_temperature_kelvin {
                                    state.color_temperature_kelvin = v;
                                    state.on = true;
                                }

                                if let Some(op) = &packet.op {
                                    for cmd in &op.command {
                                        let decoded = cmd.decode_for_sku(sku);
                                        log::debug!("Decoded: {decoded:?} for {sku}");
                                        match decoded {
                                            GoveeBlePacket::NotifyHumidifierNightlight(nl) => {
                                                state.brightness = nl.brightness;
                                                state.color = DeviceColor {
                                                    r: nl.r,
                                                    g: nl.g,
                                                    b: nl.b,
                                                };
                                                device.set_nightlight_state(nl);
                                            }
                                            GoveeBlePacket::NotifyHumidifierAutoMode(
                                                HumidifierAutoMode { target_humidity },
                                            ) => {
                                                device.set_target_humidity(
                                                    target_humidity.as_percent(),
                                                );
                                            }
                                            GoveeBlePacket::NotifyHumidifierMode(
                                                NotifyHumidifierMode { mode, param },
                                            ) => {
                                                device.set_humidifier_work_mode_and_param(
                                                    mode, param,
                                                );
                                            }
                                            GoveeBlePacket::Generic(_) => {
                                                // Ignore packets that we can't decode
                                            }
                                            GoveeBlePacket::SetHumidifierMode(_)
                                            | GoveeBlePacket::SetHumidifierNightlight(_) => {
                                                // Ignore packets that are essentially echoing
                                                // commands sent to the device
                                            }
                                            _ => {
                                                // But warn about the ones we could decode and
                                                // aren't handling here
                                                log::warn!(
                                                    "Taking no action for {decoded:?} for {sku}"
                                                );
                                            }
                                        }
                                    }
                                }

                                // Check on/off last, as we can synthesize "on"
                                // if the other fields are present
                                if let Some(on_off) = packet.state.on_off {
                                    state.on = on_off != 0;
                                    // For multi-outlet sockets the onOff value
                                    // packs each outlet into one bit, rather
                                    // than being a plain boolean.
                                    // <https://github.com/wez/govee2mqtt/issues/65>
                                    if device.socket_outlet_count().is_some() {
                                        device.set_socket_outlet_bits(on_off);
                                    }
                                }
                                device.set_iot_device_status(state);
                            }
                            state.notify_of_state_change(device_id).await?;
                        }
                    }
                    Err(err) => {
                        log::error!("Decoding IoT Packet: {err:#} {payload}");
                    }
                }
            }
            Event::Incoming(MqttPacket::ConnAck(_)) => {
                log::info!("IoT (re)connected");

                client
                    .subscribe(acct.topic.as_str(), QoS::AtMostOnce)
                    .await
                    .context("subscribe to account topic")?;
                // This logic tries to subscribe to the same data that is
                // being sent to the individual devices, but the server
                // will close the connection on us when we try this.
                if false {
                    let devices = state.devices().await;
                    for d in devices {
                        if let Some(undoc) = &d.undoc_device_info
                            && let Ok(topic) = undoc.entry.device_topic()
                        {
                            client
                                .subscribe(topic, QoS::AtMostOnce)
                                .await
                                .with_context(|| format!("subscribe to device topic {topic}"))?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// The `turn` `val` byte for a Wi-Fi smart plug/switch, matching the Govee app's
/// CmdTurn.getCmd. The high nibble selects which outlet(s) the command targets,
/// the low nibble carries their on/off bits. `outlet` is the zero-based outlet
/// index; pass 15 to address all outlets (used for single-outlet plugs, which
/// the app sends as 0xFF/0xF0).
fn socket_turn_val(outlet: u8, on: bool) -> u8 {
    let (select, bit) = match outlet {
        0 => (0x10, 0x01),
        1 => (0x20, 0x02),
        2 => (0x40, 0x04),
        _ => (0xf0, 0x0f),
    };
    select | if on { bit } else { 0 }
}

#[cfg(test)]
mod test {
    use super::socket_turn_val;

    #[test]
    fn socket_turn_values_match_app() {
        // From the Govee app's H5080-family CmdTurn.getCmd (decompiled).
        assert_eq!(socket_turn_val(15, true), 0xff); // single plug ON
        assert_eq!(socket_turn_val(15, false), 0xf0); // single plug OFF
        assert_eq!(socket_turn_val(0, true), 0x11);
        assert_eq!(socket_turn_val(0, false), 0x10);
        assert_eq!(socket_turn_val(1, true), 0x22);
        assert_eq!(socket_turn_val(1, false), 0x20);
        assert_eq!(socket_turn_val(2, true), 0x44);
        assert_eq!(socket_turn_val(2, false), 0x40);
    }
}
