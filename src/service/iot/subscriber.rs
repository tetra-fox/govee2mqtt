//! The AWS IoT MQTT subscriber loop: decode the state packets devices publish
//! and fold them into our device state. This is where per-device-type packet
//! handling accumulates (the `op.command` BLE decode match), kept apart from the
//! connection setup and the outbound control commands in the parent module.

use crate::service::state::StateHandle;
use anyhow::Context;
use govee_api::ble::{Base64HexBytes, GoveeBlePacket, HumidifierAutoMode, NotifyHumidifierMode};
use govee_api::http::from_json;
use govee_api::lan_api::{DeviceColor, DeviceStatus};
use govee_api::undoc_api::LoginAccountResponse;
use rumqttc::{AsyncClient, Event, EventLoop, Packet as MqttPacket, QoS};
use serde::Deserialize;
use std::time::Duration;

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

pub(super) async fn run_iot_subscriber(
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
                        log::trace!("{packet:?}");
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
                                        log::trace!("Decoded: {decoded:?} for {sku}");
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
                                            GoveeBlePacket::NotifyAurora(aurora) => {
                                                device.refine_aurora_from_status(aurora);
                                            }
                                            GoveeBlePacket::NotifyLaser(laser) => {
                                                device.refine_laser_from_status(laser);
                                            }
                                            GoveeBlePacket::NotifyCountdown(countdown) => {
                                                device.record_h5082_countdown(countdown);
                                            }
                                            GoveeBlePacket::NotifyTimerCount(_) => {
                                                // Per-outlet timer count; held state
                                                // wiring lands with the recurring-timer
                                                // entities in a follow-up commit.
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
                log::info!("IoT connected");

                client
                    .subscribe(acct.topic.as_str(), QoS::AtMostOnce)
                    .await
                    .context("subscribe to account topic")?;
                // Status replies land on the account topic we just subscribed
                // to; let the first poll proceed now that it won't lose them.
                state.signal_iot_ready();
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
