use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::instance::{Component, EntityInstance, component};
use crate::hass_mqtt::router::{Params, Payload, State};
use crate::hass_mqtt::topic::Topics;
use crate::hass_mqtt::work_mode::ParsedWorkMode;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::{HassClient, IdParameter};
use crate::service::state::StateHandle;
use async_trait::async_trait;
use serde::Serialize;

/// <https://www.home-assistant.io/integrations/fan.mqtt>
///
/// Govee fans (H7100..H7111) expose their speed via the platform-API workMode
/// STRUCT: a workMode enum with one entry named "FanSpeed" (value typically 1)
/// and a modeValue enum carrying the discrete speed levels. HA's fan entity
/// supports a percentage with a `speed_range_min`/`speed_range_max` divisor,
/// so we expose 1..=max as the published percentage values and let HA convert
/// to a 0-100% slider for the user. Per-SKU max comes from the quirk
/// (`fan_speed_max`); the H7107 and H7105 go to 12, others default to 8.
#[derive(Serialize, Clone, Debug)]
pub struct FanConfig {
    #[serde(flatten)]
    pub base: EntityConfig,

    pub command_topic: String,
    pub state_topic: String,

    pub percentage_command_topic: String,
    pub percentage_state_topic: String,
    pub speed_range_min: u8,
    pub speed_range_max: u8,

    pub optimistic: bool,
}

#[derive(Clone)]
pub struct Fan {
    fan: FanConfig,
    device_id: String,
}

impl Fan {
    pub async fn new(
        topics: &Topics,
        device: &ServiceDevice,
        state: &StateHandle,
    ) -> anyhow::Result<Self> {
        // ON/OFF goes through the existing powerSwitch handler the same way the
        // humidifier entity does; no separate fan-power topic needed.
        let command_topic = topics.switch_command(device, "powerSwitch");
        let state_topic = topics.fan_state(device);

        let percentage_command_topic = topics.fan_set_speed(device);
        let percentage_state_topic = topics.fan_notify_speed(device);

        let speed_range_max = device
            .resolve_quirk()
            .and_then(|q| q.fan_speed_max)
            .unwrap_or(8);

        let use_iot = device.iot_api_supported() && state.get_iot_client().await.is_some();
        let optimistic = !use_iot;

        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);

        Ok(Self {
            fan: FanConfig {
                base: EntityConfig {
                    availability,
                    availability_mode,
                    name: None,
                    device_class: None,
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id: topics.entity_id(device, "fan"),
                    entity_category: None,
                    icon: None,
                },
                command_topic,
                state_topic,
                percentage_command_topic,
                percentage_state_topic,
                speed_range_min: 1,
                speed_range_max,
                optimistic,
            },
            device_id: device.id.to_string(),
        })
    }
}

#[async_trait]
impl EntityInstance for Fan {
    fn component(&self) -> Component {
        component("fan", &self.fan.base, &self.fan)
    }

    fn device_id(&self) -> Option<&str> {
        Some(&self.device_id)
    }

    async fn notify_state(
        &self,
        device: Option<&ServiceDevice>,
        client: &HassClient,
    ) -> anyhow::Result<()> {
        let Some(device) = device else { return Ok(()) };

        match device.device_state() {
            Some(device_state) => {
                let is_on = device_state.on;
                client
                    .publish(&self.fan.state_topic, if is_on { "ON" } else { "OFF" })
                    .await?;
            }
            None => {
                // No reported state yet; leave the entity unknown rather than guessing
                return Ok(());
            }
        }

        // Speed comes out of the workMode capability: when workMode == FanSpeed,
        // modeValue is the integer speed level we publish back as the percentage.
        if let Some(cap) = device.get_state_capability_by_instance("workMode")
            && let Some(mode_value_node) = cap.state.pointer("/value/modeValue")
            && let Some(mode_value) = mode_value_node.as_i64()
        {
            client
                .publish(&self.fan.percentage_state_topic, mode_value.to_string())
                .await?;
        }

        Ok(())
    }
}

/// Receives the speed value HA publishes (an integer in
/// `[speed_range_min, speed_range_max]`) and routes it to the device's
/// FanSpeed work mode.
pub async fn mqtt_fan_set_speed(
    Payload(speed): Payload<i64>,
    Params(IdParameter { id }): Params<IdParameter>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    log::info!("mqtt_fan_set_speed: {id}: {speed}");
    let device = state.resolve_device_for_control(&id).await?;

    let work_modes = ParsedWorkMode::with_device(&device)?;
    // Different Govee appliance classes use different names for "the user-
    // controllable speed mode" on their workMode capability. The lasswellt
    // protocol reference documents:
    //   - Fans (H7101, H7107): "FanSpeed"
    //   - Air purifiers (H7120/H7122/H7123/H7124/H7127): "gearMode"
    //   - Dehumidifiers (H7151): "gearMode"
    //   - Humidifiers (H7140): "Manual"
    // Diffusers aren't documented but most likely share one of these names.
    // Try each in order; fall back to the first listed mode so a device with
    // an unknown name still gets _some_ speed control.
    let fan_mode = ["FanSpeed", "gearMode", "Manual"]
        .iter()
        .find_map(|name| work_modes.mode_by_name(name))
        .or_else(|| work_modes.modes.values().next())
        .ok_or_else(|| anyhow::anyhow!("device {id} has no work modes to drive the fan"))?;
    let mode_num = fan_mode
        .value
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("expected FanSpeed workMode value to be a number"))?;

    state.fan_set_speed(&device, mode_num, speed).await?;
    Ok(())
}
