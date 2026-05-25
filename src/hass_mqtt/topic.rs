use crate::service::device::Device as ServiceDevice;
use crate::service::hass::{topic_safe_id, topic_safe_string};

/// Builds every MQTT topic and Home Assistant unique id from a single base
/// prefix. The publish-side topics (with concrete ids) and the subscribe-side
/// route patterns (with `:param` placeholders) are defined next to each other
/// so the two sides can't drift apart. The base prefix defaults to
/// "govee2mqtt"; users migrating from an upstream install can set it to
/// "gv2mqtt" to keep their existing topics and entities.
#[derive(Clone, Debug)]
pub struct Topics {
    base: String,
}

impl Topics {
    pub fn new(base: impl Into<String>) -> Self {
        Self { base: base.into() }
    }

    // ---- service-wide topics (no device) ----

    /// All entities share this topic so the broker last-will can mark them
    /// unavailable at once
    pub fn availability(&self) -> String {
        format!("{}/availability", self.base)
    }

    pub fn oneclick(&self) -> String {
        format!("{}/oneclick", self.base)
    }

    pub fn purge_caches(&self) -> String {
        format!("{}/purge-caches", self.base)
    }

    // ---- light ----

    pub fn light_command(&self, device: &ServiceDevice) -> String {
        format!("{}/light/{}/command", self.base, topic_safe_id(device))
    }

    pub fn light_segment_command(&self, device: &ServiceDevice, segment: u32) -> String {
        format!(
            "{}/light/{}/command/{segment}",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn light_state(&self, device: &ServiceDevice) -> String {
        format!("{}/light/{}/state", self.base, topic_safe_id(device))
    }

    pub fn light_segment_state(&self, device: &ServiceDevice, segment: u32) -> String {
        format!(
            "{}/light/{}/state/{segment}",
            self.base,
            topic_safe_id(device)
        )
    }

    // ---- switch ----

    pub fn switch_command(&self, device: &ServiceDevice, instance: &str) -> String {
        format!(
            "{}/switch/{}/command/{instance}",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn switch_instance_state(&self, device: &ServiceDevice, instance: &str) -> String {
        format!(
            "{}/switch/{}/{instance}/state",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn outlet_command(&self, device: &ServiceDevice, index: u8) -> String {
        format!(
            "{}/switch/{}/outlet/{index}/command",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn outlet_state(&self, device: &ServiceDevice, index: u8) -> String {
        format!(
            "{}/switch/{}/outlet/{index}/state",
            self.base,
            topic_safe_id(device)
        )
    }

    // ---- humidifier ----

    pub fn humidifier_set_target(&self, device: &ServiceDevice) -> String {
        format!(
            "{}/humidifier/{}/set-target",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn humidifier_notify_target(&self, device: &ServiceDevice) -> String {
        format!(
            "{}/humidifier/{}/notify-target",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn humidifier_state(&self, device: &ServiceDevice) -> String {
        format!("{}/humidifier/{}/state", self.base, topic_safe_id(device))
    }

    pub fn humidifier_set_mode(&self, device: &ServiceDevice) -> String {
        format!(
            "{}/humidifier/{}/set-mode",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn humidifier_notify_mode(&self, device: &ServiceDevice) -> String {
        format!(
            "{}/humidifier/{}/notify-mode",
            self.base,
            topic_safe_id(device)
        )
    }

    // ---- number / work-mode ----

    pub fn number_command(
        &self,
        device: &ServiceDevice,
        mode_name: &str,
        mode_num: impl std::fmt::Display,
    ) -> String {
        format!(
            "{}/number/{}/command/{mode}/{mode_num}",
            self.base,
            topic_safe_id(device),
            mode = topic_safe_string(mode_name),
        )
    }

    pub fn number_state(&self, device: &ServiceDevice, mode_name: &str) -> String {
        format!(
            "{}/number/{}/state/{mode}",
            self.base,
            topic_safe_id(device),
            mode = topic_safe_string(mode_name),
        )
    }

    /// Command topic for a generic platform-API Range capability exposed as a
    /// number. The instance name is part of the path so multiple range
    /// capabilities on one device don't collide.
    pub fn capability_number_command(&self, device: &ServiceDevice, instance: &str) -> String {
        format!(
            "{}/number/{}/capability/{inst}/command",
            self.base,
            topic_safe_id(device),
            inst = topic_safe_string(instance),
        )
    }

    pub fn capability_number_state(&self, device: &ServiceDevice, instance: &str) -> String {
        format!(
            "{}/number/{}/capability/{inst}/state",
            self.base,
            topic_safe_id(device),
            inst = topic_safe_string(instance),
        )
    }

    // ---- generic mode / music ----

    /// Command topic for a generic platform-API Mode capability exposed as a
    /// select. The instance name is part of the path so multiple mode
    /// capabilities on one device don't collide.
    pub fn capability_mode_command(&self, device: &ServiceDevice, instance: &str) -> String {
        format!(
            "{}/select/{}/capability/{inst}/command",
            self.base,
            topic_safe_id(device),
            inst = topic_safe_string(instance),
        )
    }

    pub fn capability_mode_state(&self, device: &ServiceDevice, instance: &str) -> String {
        format!(
            "{}/select/{}/capability/{inst}/state",
            self.base,
            topic_safe_id(device),
            inst = topic_safe_string(instance),
        )
    }

    /// Command/state for the user's preferred music sensitivity, sent with the
    /// "Music: X" scenes.
    pub fn music_sensitivity_command(&self, device: &ServiceDevice) -> String {
        format!(
            "{}/number/{}/music-sensitivity/command",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn music_sensitivity_state(&self, device: &ServiceDevice) -> String {
        format!(
            "{}/number/{}/music-sensitivity/state",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn music_auto_color_command(&self, device: &ServiceDevice) -> String {
        format!(
            "{}/switch/{}/music-auto-color/command",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn music_auto_color_state(&self, device: &ServiceDevice) -> String {
        format!(
            "{}/switch/{}/music-auto-color/state",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn set_work_mode(&self, device: &ServiceDevice) -> String {
        format!("{}/{}/set-work-mode", self.base, topic_safe_id(device))
    }

    pub fn notify_work_mode(&self, device: &ServiceDevice) -> String {
        format!("{}/{}/notify-work-mode", self.base, topic_safe_id(device))
    }

    // ---- scene ----

    pub fn set_mode_scene(&self, device: &ServiceDevice) -> String {
        format!("{}/{}/set-mode-scene", self.base, topic_safe_id(device))
    }

    pub fn notify_mode_scene(&self, device: &ServiceDevice) -> String {
        format!("{}/{}/notify-mode-scene", self.base, topic_safe_id(device))
    }

    // ---- temperature ----

    pub fn set_temperature(&self, device: &ServiceDevice, instance: &str, units: &str) -> String {
        format!(
            "{}/{}/set-temperature/{inst}/{units}",
            self.base,
            topic_safe_id(device),
            inst = topic_safe_string(instance),
        )
    }

    pub fn advise_set_temperature(&self, device: &ServiceDevice) -> String {
        format!(
            "{}/{}/advise-set-temperature",
            self.base,
            topic_safe_id(device)
        )
    }

    // ---- diagnostics / platform data ----

    pub fn request_platform_data(&self, device: &ServiceDevice) -> String {
        format!(
            "{}/{}/request-platform-data",
            self.base,
            topic_safe_id(device)
        )
    }

    pub fn sensor_state(&self, unique_id: &str) -> String {
        format!("{}/sensor/{unique_id}/state", self.base)
    }

    pub fn sensor_attributes(&self, unique_id: &str) -> String {
        format!("{}/sensor/{unique_id}/attributes", self.base)
    }

    // ---- unique ids ----

    /// The unique id for the central "Govee2MQTT" service device, also used as
    /// its HA device identifier and as the via_device of every real device.
    pub fn service_id(&self) -> String {
        self.base.clone()
    }

    /// The HA device identifier for a real device, parented under the service
    /// device via service_id().
    pub fn device_id(&self, device: &ServiceDevice) -> String {
        format!("{}-{}", self.base, topic_safe_id(device))
    }

    /// A per-device, per-instance unique id (eg: switch instance, humidifier).
    pub fn entity_id(&self, device: &ServiceDevice, suffix: &str) -> String {
        format!("{}-{}-{suffix}", self.base, topic_safe_id(device))
    }

    pub fn light_unique_id(&self, device: &ServiceDevice, segment: Option<u32>) -> String {
        let seg = segment.map(|n| format!("-{n}")).unwrap_or_default();
        format!("{}-{}{seg}", self.base, topic_safe_id(device))
    }

    pub fn status_sensor_id(&self, device: &ServiceDevice) -> String {
        format!("sensor-{}-{}-status", topic_safe_id(device), self.base)
    }

    pub fn one_click_id(&self, simple_uuid: impl std::fmt::Display) -> String {
        format!("{}-one-click-{simple_uuid}", self.base)
    }
}

/// Subscribe-side route patterns. These carry `:param` placeholders for the
/// MQTT router to bind, and must describe the same paths the publish-side
/// methods on Topics produce. They are grouped here so the agreement between
/// the two sides is easy to check at a glance.
impl Topics {
    pub fn route_light_command(&self) -> String {
        format!("{}/light/:id/command", self.base)
    }

    pub fn route_light_segment_command(&self) -> String {
        format!("{}/light/:id/command/:segment", self.base)
    }

    pub fn route_switch_command(&self) -> String {
        format!("{}/switch/:id/command/:instance", self.base)
    }

    pub fn route_outlet_command(&self) -> String {
        format!("{}/switch/:id/outlet/:index/command", self.base)
    }

    pub fn route_request_platform_data(&self) -> String {
        format!("{}/:id/request-platform-data", self.base)
    }

    pub fn route_number_command(&self) -> String {
        format!("{}/number/:id/command/:mode_name/:work_mode", self.base)
    }

    pub fn route_capability_number_command(&self) -> String {
        format!("{}/number/:id/capability/:instance/command", self.base)
    }

    pub fn route_capability_mode_command(&self) -> String {
        format!("{}/select/:id/capability/:instance/command", self.base)
    }

    pub fn route_music_sensitivity_command(&self) -> String {
        format!("{}/number/:id/music-sensitivity/command", self.base)
    }

    pub fn route_music_auto_color_command(&self) -> String {
        format!("{}/switch/:id/music-auto-color/command", self.base)
    }

    pub fn route_humidifier_set_mode(&self) -> String {
        format!("{}/humidifier/:id/set-mode", self.base)
    }

    pub fn route_set_work_mode(&self) -> String {
        format!("{}/:id/set-work-mode", self.base)
    }

    pub fn route_humidifier_set_target(&self) -> String {
        format!("{}/humidifier/:id/set-target", self.base)
    }

    pub fn route_set_temperature(&self) -> String {
        format!("{}/:id/set-temperature/:instance/:units", self.base)
    }

    pub fn route_set_mode_scene(&self) -> String {
        format!("{}/:id/set-mode-scene", self.base)
    }
}
