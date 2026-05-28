use crate::UndocApiArguments;
use crate::service::state::StateHandle;
use anyhow::Context;
use govee_api::undoc_api::{
    DeviceEntry, GoveeUndocumentedApi, LoginAccountResponse, ParsedOneClick, ms_timestamp,
};
use rumqttc::{AsyncClient, MqttOptions, QoS, TlsConfiguration, Transport};
use serde_json::{Map, Value as JsonValue, json};
use std::time::Duration;

mod subscriber;
use subscriber::run_iot_subscriber;

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
            return Ok(self.undoc.control_device(device, msg).await?);
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
    // convert in memory rather than writing the key to disk. p12 hands back the
    // private key as PKCS#8 DER and the cert as X.509 DER; wrap each in PEM with
    // the matching RFC 7468 label (rustls-pemfile, which rumqttc uses to read
    // these, parses PKCS8 PRIVATE KEY and CERTIFICATE blocks).
    log::trace!("parsing IoT PFX key");
    let container = p12::PFX::parse(&key_bytes).context("PFX::parse")?;
    let key_pem = container
        .key_bags(&res.p12_pass)
        .context("key_bags")?
        .into_iter()
        .next()
        .map(|der| pem::encode(&pem::Pem::new("PRIVATE KEY", der)).into_bytes())
        .context("PFX contained no private key")?;
    let cert_pem = container
        .cert_bags(&res.p12_pass)
        .context("cert_bags")?
        .into_iter()
        .next()
        .map(|der| pem::encode(&pem::Pem::new("CERTIFICATE", der)).into_bytes())
        .context("PFX contained no certificate")?;

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
