use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::instance::{Component, EntityInstance, component};
use crate::hass_mqtt::topic::Topics;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::HassClient;
use async_trait::async_trait;
use govee_api::platform_api::DeviceType;
use serde::Serialize;
use serde_json::json;

/// <https://www.home-assistant.io/integrations/light.mqtt/#json-schema>
#[derive(Serialize, Clone, Debug)]
pub struct LightConfig {
    #[serde(flatten)]
    pub base: EntityConfig,
    pub schema: String,

    pub command_topic: String,
    /// The docs say that this is optional, but hass errors out if
    /// it is not passed
    pub state_topic: String,
    pub optimistic: bool,
    pub supported_color_modes: Vec<String>,
    /// Flag that defines if the light supports brightness.
    pub brightness: bool,
    /// Defines the maximum brightness value (i.e., 100%) of the MQTT device.
    pub brightness_scale: u32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Flag that defines if the light supports effects.
    pub effect: bool,
    /// The list of effects the light supports.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effect_list: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_kelvin: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_kelvin: Option<u32>,

    /// Segment lights are disabled by default: a device can have dozens, and
    /// most users only want the whole-device light. Omitted (HA enables) for
    /// the primary light.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled_by_default: Option<bool>,

    pub payload_available: String,
}

#[derive(Clone)]
pub struct DeviceLight {
    light: LightConfig,
    device_id: String,
}

#[async_trait]
impl EntityInstance for DeviceLight {
    fn component(&self) -> Component {
        component("light", &self.light.base, &self.light)
    }

    fn device_id(&self) -> Option<&str> {
        Some(&self.device_id)
    }

    async fn notify_state(
        &self,
        device: Option<&ServiceDevice>,
        client: &HassClient,
    ) -> anyhow::Result<()> {
        if self.light.optimistic {
            return Ok(());
        }

        let device = device.expect("device to exist");

        match device.device_state() {
            Some(device_state) => {
                log::trace!("LightConfig::notify_state: state is {device_state:?}");

                let is_on = device_state.light_on.unwrap_or(false);

                let light_state = if is_on {
                    if device_state.kelvin == 0 {
                        json!({
                            "state": "ON",
                            "color_mode": "rgb",
                            "color": {
                                "r": device_state.color.r,
                                "g": device_state.color.g,
                                "b": device_state.color.b,
                            },
                            "brightness": device_state.brightness,
                            "effect": device_state.scene,
                        })
                    } else {
                        json!({
                            "state": "ON",
                            "color_mode": "color_temp",
                            "brightness": device_state.brightness,
                            "color_temp_kelvin": device_state.kelvin,
                            "effect": device_state.scene,
                        })
                    }
                } else {
                    json!({"state":"OFF"})
                };

                client
                    .publish_obj(&self.light.state_topic, &light_state)
                    .await
            }
            None => {
                // TODO: mark as unavailable or something? Don't
                // want to prevent attempting to control it though,
                // as that could cause it to wake up.
                client
                    .publish_obj(&self.light.state_topic, &json!({"state":"OFF"}))
                    .await
            }
        }
    }
}

impl DeviceLight {
    pub fn for_device(
        topics: &Topics,
        device: &ServiceDevice,
        segment: Option<u32>,
        effect_list: &[String],
    ) -> Self {
        let quirk = device.resolve_quirk();
        let device_type = device.device_type();

        let command_topic = match segment {
            None => topics.light_command(device),
            Some(seg) => topics.light_segment_command(device, seg),
        };

        let icon = match segment {
            Some(_) => None,
            None if device_type == DeviceType::Light => quirk.as_ref().map(|q| q.icon.to_string()),
            None => None,
        };

        let state_topic = match segment {
            Some(seg) => topics.light_segment_state(device, seg),
            None => topics.light_state(device),
        };
        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);
        let unique_id = topics.light_unique_id(device, segment);

        let mut supported_color_modes = vec![];

        if segment.is_some() || device.supports_rgb() {
            supported_color_modes.push("rgb".to_string());
        }

        let (min_kelvin, max_kelvin) = if segment.is_some() {
            (None, None)
        } else if let Some((min, max)) = device.get_color_temperature_range() {
            supported_color_modes.push("color_temp".to_string());
            (Some(min), Some(max))
        } else {
            (None, None)
        };

        let brightness = segment.is_some()
            || quirk
                .as_ref()
                .map(|q| q.supports_brightness)
                .unwrap_or(false)
            || device
                .http_device_info
                .as_ref()
                .map(|info| info.supports_brightness())
                .unwrap_or(false);

        let name = match segment {
            Some(n) => Some(format!("Segment {:03}", n + 1)),
            None if device_type == DeviceType::Humidifier => Some("Night Light".to_string()),
            None => None,
        };

        Self {
            light: LightConfig {
                base: EntityConfig {
                    availability,
                    availability_mode,
                    name,
                    device_class: None,
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id,
                    entity_category: None,
                    icon: None,
                },
                schema: "json".to_string(),
                command_topic,
                state_topic,
                supported_color_modes,
                brightness,
                brightness_scale: 100,
                effect: true,
                effect_list: effect_list.to_vec(),
                payload_available: "online".to_string(),
                max_kelvin,
                min_kelvin,
                enabled_by_default: segment.map(|_| false),
                optimistic: segment.is_some(),
                icon,
            },
            device_id: device.id.to_string(),
        }
    }
}
