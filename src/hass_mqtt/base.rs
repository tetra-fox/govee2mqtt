use crate::hass_mqtt::topic::Topics;
use crate::service::device::Device as ServiceDevice;
use crate::version_info::govee_version;
use serde::Serialize;

const MODEL: &str = "govee2mqtt";
const URL: &str = "https://github.com/tetra-fox/govee2mqtt";

#[derive(Serialize, Clone, Debug, Default)]
pub struct EntityConfig {
    pub availability_topic: String,
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_class: Option<&'static str>,
    pub origin: Origin,
    pub device: Device,
    pub unique_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Origin {
    pub name: &'static str,
    pub sw_version: &'static str,
    pub url: &'static str,
}

impl Default for Origin {
    fn default() -> Self {
        Self {
            name: MODEL,
            sw_version: govee_version(),
            url: URL,
        }
    }
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Device {
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sw_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_area: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_device: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub identifiers: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<(String, String)>,
}

impl Device {
    pub fn for_device(topics: &Topics, device: &ServiceDevice) -> Self {
        Self {
            name: device.name(),
            manufacturer: "Govee".to_string(),
            model: device.sku.to_string(),
            sw_version: None,
            suggested_area: device.room_name().map(|s| s.to_string()),
            via_device: Some(topics.service_id()),
            identifiers: vec![
                topics.device_id(device),
                /*
                device.computed_name(),
                device.id.to_string(),
                */
            ],
            connections: vec![],
        }
    }

    pub fn this_service(topics: &Topics) -> Self {
        Self {
            name: "Govee2MQTT".to_string(),
            manufacturer: "tetra-fox".to_string(),
            model: "govee2mqtt".to_string(),
            sw_version: Some(govee_version().to_string()),
            suggested_area: None,
            via_device: None,
            identifiers: vec![topics.service_id()],
            connections: vec![],
        }
    }
}
