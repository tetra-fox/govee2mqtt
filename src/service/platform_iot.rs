//! Govee Platform API MQTT event subscriber. Devices that advertise
//! `devices.capabilities.event` (presence sensors, ice makers, dehumidifiers,
//! ...) publish notifications to a broker at mqtts://mqtt.openapi.govee.com:8883
//! that is separate from the AWS IoT broker the undocumented API uses.
//!
//! Protocol summary, from <https://developer.govee.com/reference/subscribe-device-event>:
//!  - TLS (public CA), username + password are both the API key
//!  - Topic is documented as `GA/<api-key>` in prose but the JS example in the
//!    same page subscribes to just `<api-key>`; we subscribe to both
//!  - Payload: `{sku, device, deviceName, capabilities: [{type, instance,
//!    state: [{name, value, message?}]}]}`
//!
//! For now we just decode and log the events and signal a state-change so any
//! HTTP-API state subscribers re-query. Mapping event-instance values into
//! per-device state belongs in the device modules, where the schema is known.

use crate::service::state::StateHandle;
use anyhow::Context;
use govee_api::http::from_json;
use govee_api::platform_api::DeviceCapabilityKind;
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Packet as MqttPacket, QoS, Transport};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::time::Duration;

const BROKER_HOST: &str = "mqtt.openapi.govee.com";
const BROKER_PORT: u16 = 8883;

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct EventMessage {
    sku: String,
    device: String,
    #[serde(default, rename = "deviceName")]
    device_name: String,
    #[serde(default)]
    capabilities: Vec<EventCapability>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct EventCapability {
    #[serde(rename = "type")]
    kind: DeviceCapabilityKind,
    instance: String,
    #[serde(default)]
    state: Vec<EventState>,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct EventState {
    name: String,
    #[serde(default)]
    value: JsonValue,
    #[serde(default)]
    message: Option<String>,
}

pub async fn start_platform_iot(api_key: String, state: StateHandle) -> anyhow::Result<()> {
    // client_id has to be unique per connection; the broker is EMQX-backed and
    // disconnects duplicates.
    let client_id = format!("govee2mqtt-{}", uuid::Uuid::new_v4().simple());

    let mut mqtt_options = MqttOptions::new(client_id, BROKER_HOST, BROKER_PORT);
    mqtt_options.set_credentials(&api_key, &api_key);
    mqtt_options.set_keep_alive(Duration::from_secs(60));
    mqtt_options.set_transport(Transport::tls_with_default_config());

    let (client, eventloop) = AsyncClient::new(mqtt_options, 32);

    log::info!("Connecting to platform MQTT {BROKER_HOST}:{BROKER_PORT}");
    tokio::spawn(async move {
        if let Err(err) = run_platform_iot(eventloop, state, client, api_key).await {
            log::error!("Platform MQTT loop failed: {err:#}");
        }
        log::info!("Platform MQTT loop terminated");
    });

    Ok(())
}

async fn run_platform_iot(
    mut eventloop: EventLoop,
    state: StateHandle,
    client: AsyncClient,
    api_key: String,
) -> anyhow::Result<()> {
    loop {
        let event = match eventloop.poll().await {
            Ok(event) => event,
            Err(err) => {
                // rumqttc reconnects on the next poll.
                log::warn!("Platform MQTT disconnected: {err:#}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        match event {
            Event::Incoming(MqttPacket::ConnAck(_)) => {
                log::info!("Platform MQTT (re)connected");
                // The published docs say the topic is `GA/<api-key>`, but the
                // JS example on the same page subscribes to just `<api-key>`.
                // Subscribe to both so we receive events regardless of which
                // the broker actually publishes on.
                let prefixed = format!("GA/{api_key}");
                client
                    .subscribe(&prefixed, QoS::AtLeastOnce)
                    .await
                    .with_context(|| format!("subscribe to platform topic {prefixed}"))?;
                client
                    .subscribe(&api_key, QoS::AtLeastOnce)
                    .await
                    .context("subscribe to platform topic (api-key only)")?;
            }
            Event::Incoming(MqttPacket::Publish(msg)) => {
                let payload = String::from_utf8_lossy(&msg.payload);
                log::trace!("platform-mqtt {} -> {payload}", msg.topic);

                match from_json::<EventMessage, _>(&msg.payload) {
                    Ok(event) => {
                        for cap in &event.capabilities {
                            for st in &cap.state {
                                log::info!(
                                    "Platform event: sku={} device={} instance={} {}={}{}",
                                    event.sku,
                                    event.device,
                                    cap.instance,
                                    st.name,
                                    st.value,
                                    st.message
                                        .as_deref()
                                        .map(|m| format!(" ({m})"))
                                        .unwrap_or_default(),
                                );
                            }
                        }
                        // Notify even when capabilities is empty so subscribers
                        // get a fresh state poll opportunity.
                        state.notify_of_state_change(&event.device).await?;
                    }
                    Err(err) => {
                        log::error!("Decoding platform MQTT event: {err:#} {payload}");
                    }
                }
            }
            _ => {}
        }
    }
}
