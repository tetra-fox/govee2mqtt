use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::instance::{publish_entity_config, EntityInstance};
use crate::hass_mqtt::topic::Topics;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::{camel_case_to_space_separated, HassClient};
use crate::service::state::StateHandle;
use async_trait::async_trait;
use govee_api::platform_api::DeviceCapability;
use serde::Serialize;
use serde_json::json;

#[derive(Serialize, Clone, Debug)]
pub struct SwitchConfig {
    #[serde(flatten)]
    pub base: EntityConfig,
    pub command_topic: String,
    pub state_topic: String,
}

impl SwitchConfig {
    pub async fn for_device(
        topics: &Topics,
        device: &ServiceDevice,
        instance: &DeviceCapability,
    ) -> anyhow::Result<Self> {
        let command_topic = topics.switch_command(device, &instance.instance);
        let state_topic = topics.switch_instance_state(device, &instance.instance);
        let availability_topic = topics.availability();
        let unique_id = topics.entity_id(device, &instance.instance);

        Ok(Self {
            base: EntityConfig {
                availability_topic,
                name: Some(camel_case_to_space_separated(&instance.instance)),
                device_class: None,
                origin: Origin::default(),
                device: Device::for_device(topics, device),
                unique_id,
                entity_category: None,
                icon: None,
            },
            command_topic,
            state_topic,
        })
    }

    pub async fn publish(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        publish_entity_config("switch", state, client, &self.base, self).await
    }
}

pub struct CapabilitySwitch {
    switch: SwitchConfig,
    device_id: String,
    state: StateHandle,
    instance_name: String,
}

impl CapabilitySwitch {
    pub async fn new(
        topics: &Topics,
        device: &ServiceDevice,
        state: &StateHandle,
        instance: &DeviceCapability,
    ) -> anyhow::Result<Self> {
        let switch = SwitchConfig::for_device(topics, device, instance).await?;
        Ok(Self {
            switch,
            device_id: device.id.to_string(),
            state: state.clone(),
            instance_name: instance.instance.to_string(),
        })
    }
}

/// One outlet of a multi-outlet socket (eg: H5082), exposed as a switch.
///
/// The platform API only reports a single combined `powerSwitch`, but the IoT
/// status packet packs each outlet into one bit of the `onOff` value, so we can
/// report the per-outlet state. Independent *control* isn't implemented yet
/// (see `mqtt_outlet_command`); we expose the switch now so the read path can be
/// tested and the entity is in place for the full feature.
/// <https://github.com/wez/govee2mqtt/issues/65>
pub struct OutletSwitch {
    switch: SwitchConfig,
    device_id: String,
    state: StateHandle,
    outlet_index: u8,
}

impl OutletSwitch {
    pub fn new(
        topics: &Topics,
        device: &ServiceDevice,
        state: &StateHandle,
        outlet_index: u8,
    ) -> Self {
        let switch = SwitchConfig {
            base: EntityConfig {
                availability_topic: topics.availability(),
                name: Some(
                    device
                        .socket_outlet_name(outlet_index)
                        .unwrap_or_else(|| format!("Outlet {}", outlet_index + 1)),
                ),
                device_class: Some("outlet"),
                origin: Origin::default(),
                device: Device::for_device(topics, device),
                unique_id: topics.entity_id(device, &format!("outlet-{outlet_index}")),
                entity_category: None,
                icon: Some("mdi:power-socket".to_string()),
            },
            command_topic: topics.outlet_command(device, outlet_index),
            state_topic: topics.outlet_state(device, outlet_index),
        };
        Self {
            switch,
            device_id: device.id.to_string(),
            state: state.clone(),
            outlet_index,
        }
    }
}

#[async_trait]
impl EntityInstance for OutletSwitch {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        self.switch.publish(state, client).await
    }

    async fn notify_state(&self, client: &HassClient) -> anyhow::Result<()> {
        let device = self
            .state
            .device_by_id(&self.device_id)
            .await
            .expect("device to exist");

        // No reported state yet; leave the entity unknown rather than guessing
        if let Some(on) = device.socket_outlet_state(self.outlet_index) {
            client
                .publish(&self.switch.state_topic, if on { "ON" } else { "OFF" })
                .await?;
        }
        Ok(())
    }
}

/// The single power switch for a plug/switch device (eg: H5080, H5083) that we
/// know only from a quirk: the platform API returns no metadata for it, so
/// there is no `powerSwitch` capability to drive `CapabilitySwitch`. We
/// synthesize the same `powerSwitch` topics from the device identity, routing
/// control through the existing switch command handler.
pub struct PowerSwitch {
    switch: SwitchConfig,
    device_id: String,
    state: StateHandle,
}

impl PowerSwitch {
    pub fn new(topics: &Topics, device: &ServiceDevice, state: &StateHandle) -> Self {
        let switch = SwitchConfig {
            base: EntityConfig {
                availability_topic: topics.availability(),
                name: Some("Power".to_string()),
                device_class: Some("outlet"),
                origin: Origin::default(),
                device: Device::for_device(topics, device),
                unique_id: topics.entity_id(device, "powerSwitch"),
                entity_category: None,
                icon: None,
            },
            command_topic: topics.switch_command(device, "powerSwitch"),
            state_topic: topics.switch_instance_state(device, "powerSwitch"),
        };
        Self {
            switch,
            device_id: device.id.to_string(),
            state: state.clone(),
        }
    }
}

#[async_trait]
impl EntityInstance for PowerSwitch {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        self.switch.publish(state, client).await
    }

    async fn notify_state(&self, client: &HassClient) -> anyhow::Result<()> {
        let device = self
            .state
            .device_by_id(&self.device_id)
            .await
            .expect("device to exist");

        // Leave the entity unknown until we have a reported state
        if let Some(device_state) = device.device_state() {
            client
                .publish(
                    &self.switch.state_topic,
                    if device_state.on { "ON" } else { "OFF" },
                )
                .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl EntityInstance for CapabilitySwitch {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        self.switch.publish(state, client).await
    }

    async fn notify_state(&self, client: &HassClient) -> anyhow::Result<()> {
        let device = self
            .state
            .device_by_id(&self.device_id)
            .await
            .expect("device to exist");

        if self.instance_name == "powerSwitch" {
            if let Some(state) = device.device_state() {
                client
                    .publish(
                        &self.switch.state_topic,
                        if state.on { "ON" } else { "OFF" },
                    )
                    .await?;
            }
            return Ok(());
        }

        // TODO: currently, Govee don't return any meaningful data on
        // additional states. When they do, we'll need to start reporting
        // it here, but we'll also need to start polling it from the
        // platform API in order for it to even be available here.
        // Until then, the switch will show in the hass UI with an
        // unknown state but provide you with separate on and off push
        // buttons so that you can at least send the commands to the device.
        // <https://developer.govee.com/discuss/6596e84c901fb900312d5968>

        if let Some(cap) = device.get_state_capability_by_instance(&self.instance_name) {
            match cap.state.pointer("/value").and_then(|v| v.as_i64()) {
                Some(n) => {
                    return client
                        .publish(&self.switch.state_topic, if n != 0 { "ON" } else { "OFF" })
                        .await;
                }
                None => {
                    if cap.state.pointer("/value") == Some(&json!("")) {
                        log::trace!(
                            "CapabilitySwitch::notify_state ignore useless \
                                            empty string state for {cap:?}"
                        );
                    } else {
                        log::warn!("CapabilitySwitch::notify_state: Do something with {cap:#?}");
                    }
                    return Ok(());
                }
            }
        }
        log::trace!(
            "CapabilitySwitch::notify_state: didn't find state for {device} {instance}",
            instance = self.instance_name
        );
        Ok(())
    }
}
