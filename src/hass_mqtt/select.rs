use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::instance::{publish_entity_config, EntityInstance};
use crate::hass_mqtt::topic::Topics;
use crate::hass_mqtt::work_mode::ParsedWorkMode;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::{camel_case_to_space_separated, HassClient, IdParameter};
use crate::service::state::StateHandle;
use anyhow::{anyhow, Context};
use govee_api::platform_api::{DeviceCapability, DeviceParameters};
use mosquitto_rs::router::{Params, Payload, State};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Serialize, Clone, Debug)]
pub struct SelectConfig {
    #[serde(flatten)]
    pub base: EntityConfig,

    pub command_topic: String,
    pub options: Vec<String>,
    pub state_topic: String,
}

impl SelectConfig {
    pub async fn publish(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        publish_entity_config("select", state, client, &self.base, self).await
    }
}

pub struct WorkModeSelect {
    select: SelectConfig,
    device_id: String,
    state: StateHandle,
}

impl WorkModeSelect {
    pub fn new(
        topics: &Topics,
        device: &ServiceDevice,
        work_modes: &ParsedWorkMode,
        state: &StateHandle,
    ) -> Self {
        let command_topic = topics.set_work_mode(device);
        let state_topic = topics.notify_work_mode(device);
        let availability_topic = topics.availability();
        let unique_id = topics.entity_id(device, "workMode");

        Self {
            select: SelectConfig {
                base: EntityConfig {
                    availability_topic,
                    name: Some("Mode".to_string()),
                    device_class: None,
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id,
                    entity_category: None,
                    icon: None,
                },
                command_topic,
                state_topic,
                options: work_modes.get_mode_names(),
            },
            device_id: device.id.to_string(),
            state: state.clone(),
        }
    }
}

#[async_trait::async_trait]
impl EntityInstance for WorkModeSelect {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        self.select.publish(state, client).await
    }

    async fn notify_state(&self, client: &HassClient) -> anyhow::Result<()> {
        let device = self
            .state
            .device_by_id(&self.device_id)
            .await
            .expect("device to exist");

        if let Some(mode_value) = device.humidifier_work_mode {
            if let Ok(work_mode) = ParsedWorkMode::with_device(&device) {
                let mode_value_json = json!(mode_value);
                if let Some(mode) = work_mode.mode_for_value(&mode_value_json) {
                    client
                        .publish(&self.select.state_topic, mode.name.to_string())
                        .await?;
                }
            }
        } else {
            let work_modes = ParsedWorkMode::with_device(&device)?;

            if let Some(cap) = device.get_state_capability_by_instance("workMode") {
                if let Some(mode_num) = cap.state.pointer("/value/workMode") {
                    if let Some(mode) = work_modes.mode_for_value(mode_num) {
                        return client
                            .publish(&self.select.state_topic, mode.name.to_string())
                            .await;
                    }
                }
            }
        }
        Ok(())
    }
}

pub struct SceneModeSelect {
    select: SelectConfig,
    device_id: String,
    state: StateHandle,
}

impl SceneModeSelect {
    pub async fn new(
        topics: &Topics,
        device: &ServiceDevice,
        state: &StateHandle,
    ) -> anyhow::Result<Option<Self>> {
        let scenes = state.device_list_scenes(device).await?;
        if scenes.is_empty() {
            return Ok(None);
        }

        let command_topic = topics.set_mode_scene(device);
        let state_topic = topics.notify_mode_scene(device);
        let availability_topic = topics.availability();
        let unique_id = topics.entity_id(device, "mode-scene");

        Ok(Some(Self {
            select: SelectConfig {
                base: EntityConfig {
                    availability_topic,
                    name: Some("Mode/Scene".to_string()),
                    device_class: None,
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id,
                    entity_category: None,
                    icon: None,
                },
                command_topic,
                state_topic,
                options: scenes,
            },
            device_id: device.id.to_string(),
            state: state.clone(),
        }))
    }
}

#[async_trait::async_trait]
impl EntityInstance for SceneModeSelect {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        self.select.publish(state, client).await
    }

    async fn notify_state(&self, client: &HassClient) -> anyhow::Result<()> {
        let device = self
            .state
            .device_by_id(&self.device_id)
            .await
            .expect("device to exist");

        if let Some(device_state) = device.device_state() {
            client
                .publish(
                    &self.select.state_topic,
                    device_state.scene.as_deref().unwrap_or(""),
                )
                .await?;
        }

        Ok(())
    }
}

pub async fn mqtt_set_mode_scene(
    Payload(scene): Payload<String>,
    Params(IdParameter { id }): Params<IdParameter>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    let device = state.resolve_device_for_control(&id).await?;

    state
        .device_set_scene(&device, &scene)
        .await
        .context("mqtt_set_mode_scene: state.device_set_scene")?;

    Ok(())
}

/// A generic platform-API Mode capability (eg: `nightlightScene`) exposed as a
/// Home Assistant select. The option labels are the enum option names; the
/// value sent on selection is the corresponding enum value. Control is sent
/// through the platform API's generic device_control path.
pub struct CapabilityModeSelect {
    select: SelectConfig,
    device_id: String,
    state: StateHandle,
    instance_name: String,
}

impl CapabilityModeSelect {
    /// Returns None if the capability isn't an enum we can present as options.
    pub fn new(
        topics: &Topics,
        device: &ServiceDevice,
        state: &StateHandle,
        cap: &DeviceCapability,
    ) -> Option<Self> {
        let options: Vec<String> = match &cap.parameters {
            Some(DeviceParameters::Enum { options }) => {
                options.iter().map(|o| o.name.to_string()).collect()
            }
            _ => return None,
        };
        if options.is_empty() {
            return None;
        }

        let command_topic = topics.capability_mode_command(device, &cap.instance);
        let state_topic = topics.capability_mode_state(device, &cap.instance);
        let unique_id = topics.entity_id(device, &format!("{}-mode", cap.instance));

        Some(Self {
            select: SelectConfig {
                base: EntityConfig {
                    availability_topic: topics.availability(),
                    name: Some(camel_case_to_space_separated(&cap.instance)),
                    device_class: None,
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id,
                    entity_category: None,
                    icon: None,
                },
                command_topic,
                state_topic,
                options,
            },
            device_id: device.id.to_string(),
            state: state.clone(),
            instance_name: cap.instance.to_string(),
        })
    }
}

#[async_trait::async_trait]
impl EntityInstance for CapabilityModeSelect {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        self.select.publish(state, client).await
    }

    async fn notify_state(&self, client: &HassClient) -> anyhow::Result<()> {
        let device = self
            .state
            .device_by_id(&self.device_id)
            .await
            .expect("device to exist");

        // Map the reported enum value back to its option name, if Govee
        // reports state for this instance.
        let reported = device
            .get_state_capability_by_instance(&self.instance_name)
            .and_then(|s| s.state.pointer("/value").cloned());
        let cap = device.get_capability_by_instance(&self.instance_name);

        if let (Some(value), Some(cap)) = (reported, cap) {
            if let Some(DeviceParameters::Enum { options }) = &cap.parameters {
                if let Some(opt) = options.iter().find(|o| o.value == value) {
                    return client
                        .publish(&self.select.state_topic, opt.name.to_string())
                        .await;
                }
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
pub struct IdAndInstance {
    id: String,
    instance: String,
}

/// HASS selected an option on a generic capability mode select.
pub async fn mqtt_set_capability_mode(
    Payload(option_name): Payload<String>,
    Params(IdAndInstance { id, instance }): Params<IdAndInstance>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    log::info!("{instance} for {id}: {option_name}");
    let device = state.resolve_device_for_control(&id).await?;

    let cap = device
        .get_capability_by_instance(&instance)
        .ok_or_else(|| anyhow!("device {id} has no capability {instance}"))?;

    let value = match &cap.parameters {
        Some(DeviceParameters::Enum { options }) => options
            .iter()
            .find(|o| o.name == option_name)
            .map(|o| o.value.clone())
            .ok_or_else(|| anyhow!("{instance} has no option named {option_name}"))?,
        _ => anyhow::bail!("{instance} is not an enum capability"),
    };

    state.device_control(&device, cap, value).await?;

    Ok(())
}
