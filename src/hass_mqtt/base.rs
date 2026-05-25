use crate::hass_mqtt::topic::Topics;
use crate::service::device::Device as ServiceDevice;
use crate::version_info::govee_version;
use serde::Serialize;

const MODEL: &str = "govee2mqtt";
const URL: &str = "https://github.com/tetra-fox/govee2mqtt";

#[derive(Serialize, Clone, Debug, Default)]
pub struct EntityConfig {
    /// The availability topics an entity tracks. Bridge-owned entities list
    /// only the global topic; real-device entities additionally list their
    /// per-device topic, with `availability_mode: all` so the entity is online
    /// only when both the bridge and the device are.
    pub availability: Vec<Availability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_mode: Option<&'static str>,
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
pub struct Availability {
    pub topic: String,
}

impl EntityConfig {
    /// Availability for an entity owned by the bridge device itself (Version,
    /// Purge Caches, scenes): just the global topic, which the broker last-will
    /// flips offline when the bridge dies.
    pub fn global_availability(topics: &Topics) -> (Vec<Availability>, Option<&'static str>) {
        (
            vec![Availability {
                topic: topics.availability(),
            }],
            None,
        )
    }

    /// Availability for a real-device entity: the global bridge topic plus the
    /// device's own topic, both required (`availability_mode: all`). The device
    /// topic is driven by Device::availability_status.
    pub fn device_availability(
        topics: &Topics,
        device: &ServiceDevice,
    ) -> (Vec<Availability>, Option<&'static str>) {
        (
            vec![
                Availability {
                    topic: topics.availability(),
                },
                Availability {
                    topic: topics.device_availability(device),
                },
            ],
            Some("all"),
        )
    }
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
    pub hw_version: Option<String>,
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
        // Firmware/hardware versions come from the undoc device list
        // (versionSoft/versionHard); devices known only via the platform API
        // don't carry them.
        let entry = device.undoc_device_info.as_ref().map(|info| &info.entry);
        Self {
            name: device.name(),
            manufacturer: "Govee".to_string(),
            model: device.sku.to_string(),
            sw_version: entry.map(|e| e.version_soft.clone()),
            hw_version: entry.map(|e| e.version_hard.clone()),
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
            hw_version: None,
            suggested_area: None,
            via_device: None,
            identifiers: vec![topics.service_id()],
            connections: vec![],
        }
    }
}
