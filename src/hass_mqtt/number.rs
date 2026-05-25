use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::instance::{EntityInstance, publish_entity_config};
use crate::hass_mqtt::router::{Params, Payload, State};
use crate::hass_mqtt::topic::Topics;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::{HassClient, camel_case_to_space_separated, topic_safe_string};
use crate::service::state::StateHandle;
use anyhow::anyhow;
use async_trait::async_trait;
use govee_api::platform_api::{DeviceCapability, DeviceParameters, IntegerRange};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::ops::Range;

#[derive(Serialize, Clone, Debug)]
pub struct NumberConfig {
    #[serde(flatten)]
    pub base: EntityConfig,

    pub command_topic: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f32>,
    pub step: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_of_measurement: Option<String>,
}

impl NumberConfig {
    pub async fn publish(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        publish_entity_config("number", state, client, &self.base, self).await
    }

    pub async fn notify_state(&self, client: &HassClient, value: &str) -> anyhow::Result<()> {
        client
            .publish(
                self.state_topic
                    .as_deref()
                    .ok_or_else(|| anyhow!("number has no state_topic"))?,
                value,
            )
            .await
    }
}

pub struct WorkModeNumber {
    number: NumberConfig,
    device_id: String,
    state: StateHandle,
    mode_name: String,
    work_mode: JsonValue,
}

impl WorkModeNumber {
    pub fn new(
        topics: &Topics,
        device: &ServiceDevice,
        state: &StateHandle,
        label: String,
        mode_name: &str,
        work_mode: JsonValue,
        range: Option<Range<i64>>,
    ) -> Self {
        let mode_num = work_mode
            .as_i64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "work-mode-was-not-int".to_string());
        let command_topic = topics.number_command(device, mode_name, &mode_num);
        let state_topic = topics.number_state(device, mode_name);

        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);
        let unique_id = topics.entity_id(
            device,
            &format!("{mode}-number", mode = topic_safe_string(mode_name)),
        );

        Self {
            number: NumberConfig {
                base: EntityConfig {
                    availability,
                    availability_mode,
                    name: Some(label),
                    device_class: None,
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id,
                    entity_category: None,
                    icon: None,
                },
                command_topic,
                state_topic: Some(state_topic),
                min: range.as_ref().map(|r| r.start as f32).or(Some(0.)),
                max: range
                    .as_ref()
                    .map(|r| r.end.saturating_sub(1) as f32)
                    .or(Some(255.)),
                step: 1f32,
                unit_of_measurement: None,
            },
            device_id: device.id.to_string(),
            state: state.clone(),
            mode_name: mode_name.to_string(),
            work_mode,
        }
    }
}

#[async_trait]
impl EntityInstance for WorkModeNumber {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        self.number.publish(state, client).await
    }

    async fn notify_state(&self, client: &HassClient) -> anyhow::Result<()> {
        let state_topic = self
            .number
            .state_topic
            .as_ref()
            .ok_or_else(|| anyhow!("state_topic is None!?"))?;

        let device = self
            .state
            .device_by_id(&self.device_id)
            .await
            .expect("device to exist");

        if let Some(cap) = device.get_state_capability_by_instance("workMode")
            && let Some(work_mode) = cap.state.pointer("/value/workMode")
            && *work_mode == self.work_mode
        {
            // The current mode matches us, so it is valid to
            // read the current parameter for that mode

            if let Some(value) = cap.state.pointer("/value/modeValue")
                && let Some(n) = value.as_i64()
            {
                client.publish(state_topic, n.to_string()).await?;
                return Ok(());
            }
        }

        if let Some(work_mode) = self.work_mode.as_i64() {
            // FIXME: assuming humidifier, rename that field?
            if let Some(n) = device.humidifier_param_by_mode.get(&(work_mode as u8)) {
                client.publish(state_topic, n.to_string()).await?;
                return Ok(());
            }
        }

        // We might get some data to report later, so this is just debug for now
        log::debug!(
            "Don't know how to report state for {} workMode {} value",
            self.device_id,
            self.mode_name
        );

        Ok(())
    }
}

/// A generic platform-API Range capability (eg: a humidifier's mist level, a
/// purifier's fan speed) exposed as a Home Assistant number. Control is sent
/// through the platform API's generic device_control path; state is read back
/// from the platform device state when Govee reports it.
pub struct CapabilityNumber {
    number: NumberConfig,
    device_id: String,
    state: StateHandle,
    instance_name: String,
}

impl CapabilityNumber {
    pub fn new(
        topics: &Topics,
        device: &ServiceDevice,
        state: &StateHandle,
        cap: &DeviceCapability,
    ) -> Self {
        let (min, max, unit) = match &cap.parameters {
            Some(DeviceParameters::Integer {
                range: IntegerRange { min, max, .. },
                unit,
            }) => (*min as f32, *max as f32, unit.clone()),
            _ => (0., 255., None),
        };

        let command_topic = topics.capability_number_command(device, &cap.instance);
        let state_topic = topics.capability_number_state(device, &cap.instance);
        let unique_id = topics.entity_id(
            device,
            &format!("{inst}-number", inst = topic_safe_string(&cap.instance)),
        );
        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);

        Self {
            number: NumberConfig {
                base: EntityConfig {
                    availability,
                    availability_mode,
                    name: Some(camel_case_to_space_separated(&cap.instance)),
                    device_class: None,
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id,
                    entity_category: None,
                    icon: None,
                },
                command_topic,
                state_topic: Some(state_topic),
                min: Some(min),
                max: Some(max),
                step: 1f32,
                unit_of_measurement: unit,
            },
            device_id: device.id.to_string(),
            state: state.clone(),
            instance_name: cap.instance.to_string(),
        }
    }
}

#[async_trait]
impl EntityInstance for CapabilityNumber {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        self.number.publish(state, client).await
    }

    async fn notify_state(&self, client: &HassClient) -> anyhow::Result<()> {
        let state_topic = self
            .number
            .state_topic
            .as_ref()
            .ok_or_else(|| anyhow!("state_topic is None!?"))?;

        let device = self
            .state
            .device_by_id(&self.device_id)
            .await
            .expect("device to exist");

        if let Some(cap) = device.get_state_capability_by_instance(&self.instance_name)
            && let Some(n) = cap.state.pointer("/value").and_then(|v| v.as_f64())
        {
            client.publish(state_topic, n.to_string()).await?;
            return Ok(());
        }

        // Govee doesn't always report a value for these; leave the entity
        // unknown rather than guessing, matching CapabilitySwitch.
        log::trace!(
            "CapabilityNumber::notify_state: no state for {device} {instance}",
            instance = self.instance_name
        );
        Ok(())
    }
}

#[derive(Deserialize)]
pub struct IdAndInstance {
    id: String,
    instance: String,
}

/// HASS set a value on a generic capability number.
pub async fn mqtt_capability_number_command(
    Payload(value): Payload<f64>,
    Params(IdAndInstance { id, instance }): Params<IdAndInstance>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    log::info!("{instance} for {id}: {value}");
    let device = state.resolve_device_for_control(&id).await?;

    let cap = device
        .get_capability_by_instance(&instance)
        .ok_or_else(|| anyhow!("device {id} has no capability {instance}"))?;

    // Govee range values are integers; round to the nearest before sending.
    state
        .device_control(&device, cap, value.round() as i64)
        .await?;

    Ok(())
}

use crate::service::hass::IdParameter;

/// The user's preferred music sensitivity, exposed as a number. Govee never
/// reports this back, so the state we publish is whatever the user last set
/// (defaulting to 100). It takes effect the next time a "Music: X" scene is
/// selected.
pub struct MusicSensitivityNumber {
    number: NumberConfig,
    device_id: String,
    state: StateHandle,
}

impl MusicSensitivityNumber {
    pub fn new(topics: &Topics, device: &ServiceDevice, state: &StateHandle) -> Self {
        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);
        Self {
            number: NumberConfig {
                base: EntityConfig {
                    availability,
                    availability_mode,
                    name: Some("Music Sensitivity".to_string()),
                    device_class: None,
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id: topics.entity_id(device, "music-sensitivity"),
                    entity_category: Some("config".to_string()),
                    icon: Some("mdi:music-note".to_string()),
                },
                command_topic: topics.music_sensitivity_command(device),
                state_topic: Some(topics.music_sensitivity_state(device)),
                min: Some(0.),
                max: Some(100.),
                step: 1f32,
                unit_of_measurement: Some("%".to_string()),
            },
            device_id: device.id.to_string(),
            state: state.clone(),
        }
    }
}

#[async_trait]
impl EntityInstance for MusicSensitivityNumber {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        self.number.publish(state, client).await
    }

    async fn notify_state(&self, client: &HassClient) -> anyhow::Result<()> {
        let device = self
            .state
            .device_by_id(&self.device_id)
            .await
            .expect("device to exist");
        self.number
            .notify_state(client, &device.music_sensitivity().to_string())
            .await
    }
}

pub async fn mqtt_music_sensitivity_command(
    Payload(value): Payload<f64>,
    Params(IdParameter { id }): Params<IdParameter>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    log::info!("music sensitivity for {id}: {value}");
    let device = state.resolve_device_for_control(&id).await?;
    state
        .device_mut(&device.sku, &device.id)
        .await
        .set_music_sensitivity(value.round().clamp(0., 100.) as u8);
    state.notify_of_state_change(&device.id).await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct IdAndModeName {
    id: String,
    mode_name: String,
    work_mode: String,
}

pub async fn mqtt_number_command(
    Payload(value): Payload<i64>,
    Params(IdAndModeName {
        id,
        mode_name,
        work_mode,
    }): Params<IdAndModeName>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    log::info!("{mode_name} for {id}: {value}");
    let work_mode: i64 = work_mode.parse()?;
    let device = state.resolve_device_for_control(&id).await?;

    state
        .humidifier_set_parameter(&device, work_mode, value)
        .await?;

    Ok(())
}
