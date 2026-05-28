use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::instance::{Component, EntityInstance, component};
use crate::hass_mqtt::router::{Params, Payload, State};
use crate::hass_mqtt::topic::Topics;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::{HassClient, IdParameter, entity_display_name};
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
        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);
        let unique_id = topics.entity_id(device, &instance.instance);

        Ok(Self {
            base: EntityConfig {
                availability,
                availability_mode,
                name: Some(entity_display_name(&instance.instance)),
                device_class: None,
                origin: Origin::default(),
                device: Device::for_device(topics, device),
                unique_id,
                entity_category: govee_api::ble::projector_entity_category(&instance.instance)
                    .unwrap_or(None),
                icon: None,
            },
            command_topic,
            state_topic,
        })
    }

    pub fn component(&self) -> Component {
        component("switch", &self.base, self)
    }
}

pub struct CapabilitySwitch {
    switch: SwitchConfig,
    device_id: String,
    instance_name: String,
}

impl CapabilitySwitch {
    pub async fn new(
        topics: &Topics,
        device: &ServiceDevice,
        instance: &DeviceCapability,
    ) -> anyhow::Result<Self> {
        let switch = SwitchConfig::for_device(topics, device, instance).await?;
        Ok(Self {
            switch,
            device_id: device.id.to_string(),
            instance_name: instance.instance.to_string(),
        })
    }
}

/// One outlet of a multi-outlet socket (eg: H5082), exposed as a switch.
///
/// The IoT status packet packs each outlet into one bit of the `onOff` value
/// (read via `socket_outlet_state`), and control goes back over IoT via
/// `mqtt_outlet_command` -> `socket_turn` -> `iot.set_socket_power`. Owned
/// devices also expose `powerSwitch` and per-outlet `socketToggleN`
/// capabilities through the platform API; the enumerator skips those so they
/// don't double up with the entities created here.
/// <https://github.com/wez/govee2mqtt/issues/65>
pub struct OutletSwitch {
    switch: SwitchConfig,
    device_id: String,
    outlet_index: u8,
}

impl OutletSwitch {
    pub fn new(topics: &Topics, device: &ServiceDevice, outlet_index: u8) -> Self {
        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);
        let switch = SwitchConfig {
            base: EntityConfig {
                availability,
                availability_mode,
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
            outlet_index,
        }
    }
}

#[async_trait]
impl EntityInstance for OutletSwitch {
    fn component(&self) -> Component {
        self.switch.component()
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
}

impl PowerSwitch {
    pub fn new(topics: &Topics, device: &ServiceDevice) -> Self {
        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);
        let switch = SwitchConfig {
            base: EntityConfig {
                availability,
                availability_mode,
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
        }
    }
}

#[async_trait]
impl EntityInstance for PowerSwitch {
    fn component(&self) -> Component {
        self.switch.component()
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

/// The user's preferred music auto-color toggle, exposed as a switch. Like the
/// sensitivity number, Govee never reports this back, so the published state is
/// whatever the user last set (defaulting to on). It takes effect the next time
/// a "Music: X" scene is selected.
pub struct MusicAutoColorSwitch {
    switch: SwitchConfig,
    device_id: String,
}

impl MusicAutoColorSwitch {
    pub fn new(topics: &Topics, device: &ServiceDevice) -> Self {
        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);
        let switch = SwitchConfig {
            base: EntityConfig {
                availability,
                availability_mode,
                name: Some("Music Auto Color".to_string()),
                device_class: None,
                origin: Origin::default(),
                device: Device::for_device(topics, device),
                unique_id: topics.entity_id(device, "music-auto-color"),
                entity_category: Some("config".to_string()),
                icon: Some("mdi:palette".to_string()),
            },
            command_topic: topics.music_auto_color_command(device),
            state_topic: topics.music_auto_color_state(device),
        };
        Self {
            switch,
            device_id: device.id.to_string(),
        }
    }
}

#[async_trait]
impl EntityInstance for MusicAutoColorSwitch {
    fn component(&self) -> Component {
        self.switch.component()
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
        client
            .publish(
                &self.switch.state_topic,
                if device.music_auto_color() {
                    "ON"
                } else {
                    "OFF"
                },
            )
            .await
    }
}

pub async fn mqtt_music_auto_color_command(
    Payload(command): Payload<String>,
    Params(IdParameter { id }): Params<IdParameter>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    log::info!("music auto color for {id}: {command}");
    let on = match command.as_str() {
        "ON" | "on" => true,
        "OFF" | "off" => false,
        _ => anyhow::bail!("invalid {command} for {id}"),
    };
    let device = state.resolve_device_for_control(&id).await?;
    state
        .device_mut(&device.sku, &device.id)
        .await
        .set_music_auto_color(on);
    state.notify_of_state_change(&device.id).await?;
    Ok(())
}

#[async_trait]
impl EntityInstance for CapabilitySwitch {
    fn component(&self) -> Component {
        self.switch.component()
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
