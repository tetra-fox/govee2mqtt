use crate::commands::serve::{POLL_INTERVAL, availability_timeout};
use crate::service::quirks::{BULB, Quirk, resolve_quirk};
use chrono::{DateTime, Utc};
use govee_api::ble::{
    NotifyAurora, NotifyHumidifierNightlightParams, NotifyLaser, ProjectorSettings, SetAuroraLaser,
    SetAutoOff,
};
use govee_api::lan_api::{DeviceColor, DeviceStatus as LanDeviceStatus, LanDevice};
use govee_api::platform_api::{
    DeviceCapability, DeviceCapabilityState, DeviceType, HttpDeviceInfo, HttpDeviceState,
};
use govee_api::undoc_api::GoveeUndocumentedApi;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::net::IpAddr;

/// How reachable a device is, based on how recently it last reported state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reachability {
    /// Reported state recently; reachable.
    Available,
    /// Had state once, but it has gone stale past the poll threshold.
    Missing,
    /// Never reported any state.
    Unknown,
}

impl Reachability {
    /// The text shown by the Status diagnostic sensor.
    pub fn as_status_text(self) -> &'static str {
        match self {
            Self::Available => "Available",
            Self::Missing => "Missing",
            Self::Unknown => "Unknown",
        }
    }

    /// The MQTT availability payload. Only a device we have heard from recently
    /// is online; stale or never-seen devices are offline so home assistant
    /// greys out their entities.
    pub fn as_mqtt_payload(self) -> &'static str {
        match self {
            Self::Available => "online",
            Self::Missing | Self::Unknown => "offline",
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct Device {
    pub sku: String,
    pub id: String,

    /// Probed LAN device information, found either via discovery
    /// or explicit probing by IP address
    pub lan_device: Option<LanDevice>,
    pub last_lan_device_update: Option<DateTime<Utc>>,

    pub lan_device_status: Option<LanDeviceStatus>,
    pub last_lan_device_status_update: Option<DateTime<Utc>>,

    pub http_device_info: Option<HttpDeviceInfo>,
    pub last_http_device_update: Option<DateTime<Utc>>,

    pub http_device_state: Option<HttpDeviceState>,
    pub last_http_device_state_update: Option<DateTime<Utc>>,

    pub undoc_device_info: Option<UndocDeviceInfo>,
    pub last_undoc_device_info_update: Option<DateTime<Utc>>,

    pub iot_device_status: Option<LanDeviceStatus>,
    pub last_iot_device_status_update: Option<DateTime<Utc>>,

    /// For multi-outlet sockets, the most recent raw onOff value from the
    /// IoT status packet. Each outlet occupies one bit. See
    /// <https://github.com/wez/govee2mqtt/issues/65>
    pub socket_outlet_bits: Option<u8>,

    pub nightlight_state: Option<NotifyHumidifierNightlightParams>,
    pub target_humidity_percent: Option<u8>,
    pub humidifier_work_mode: Option<u8>,
    pub humidifier_param_by_mode: HashMap<u8, u8>,

    /// Held aurora+laser state for projectors like the H6093. The aurora/laser
    /// controls share a single write frame that carries the full state, so a
    /// single-field change has to re-send everything; we hold the current state
    /// here, seed it from our own writes, and refine the fields we can decode
    /// from the device's status frames (aurora from aa 11, laser on/off from
    /// aa 34). None until the first status decode or write.
    pub aurora_laser_state: Option<SetAuroraLaser>,

    /// Held auto-off state for projectors like the H6093. Auto-off enable, the
    /// "stop playing sound" sub-option, and the timeout minutes share one write
    /// frame, so a single-field change re-sends all three; held here and seeded
    /// from our own writes. None until the first write.
    pub auto_off_state: Option<SetAutoOff>,

    /// Held last-written state for the H6093's standalone settings toggles. The
    /// device doesn't report them and they aren't in common-datas, so this is the
    /// only source for their HA state; each field is None until first written.
    pub projector_settings: ProjectorSettings,

    /// Whether the aurora/laser state has been seeded from common-datas yet.
    /// Tracked separately from `aurora_laser_state.is_some()` because status
    /// refinement (aa 11/34) creates a held state from device on/off before the
    /// seed runs; without this flag that would skip the seed and leave the blob
    /// with no brightness or colors, so a layer toggled on would be invisible.
    pub aurora_laser_seeded: bool,

    pub last_polled: Option<DateTime<Utc>>,

    active_scene: Option<ActiveSceneInfo>,

    /// User-chosen sensitivity/auto-color for the "Music: X" scenes. Govee
    /// requires these to be sent together with the music mode and never reports
    /// them back, so we hold the user's preference here and send it when a music
    /// scene is selected. None means use the defaults (100 / on).
    music_sensitivity: Option<u8>,
    music_auto_color: Option<bool>,
}

impl std::fmt::Display for Device {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{} ({} {})", self.name(), self.id, self.sku)
    }
}

/// Govee doesn't report the active scene or music mode,
/// so we maintain our own idea of it, clearing it when
/// the color of the light is changed
#[derive(Clone, Debug)]
struct ActiveSceneInfo {
    pub name: String,
    pub color: govee_api::lan_api::DeviceColor,
    pub kelvin: u32,
}

/// Represents the device state; synthesized from the various
/// sources of facts that we have in the Device
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeviceState {
    /// Whether the device is powered on
    pub on: bool,
    /// Whether the light function of the device is powered on
    pub light_on: Option<bool>,

    /// Whether the device is connected to the Govee cloud
    pub online: Option<bool>,

    /// The color temperature in kelvin
    pub kelvin: u32,

    /// The color
    pub color: govee_api::lan_api::DeviceColor,

    /// The brightness in percent (0-100)
    pub brightness: u8,

    /// The active effect mode, if known
    pub scene: Option<String>,

    /// Where the information came from
    pub source: &'static str,
    pub updated: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UndocDeviceInfo {
    pub room_name: Option<String>,
    pub entry: govee_api::undoc_api::DeviceEntry,
}

impl Device {
    /// Create a new device given just its sku and id.
    /// No other facts are known or reflected by it at this time;
    /// they will need to be added by the caller.
    pub fn new<S: Into<String>, I: Into<String>>(sku: S, id: I) -> Self {
        Self {
            sku: sku.into(),
            id: id.into(),
            ..Self::default()
        }
    }

    /// Returns the device name; either the name defined in the Govee App,
    /// or, if we don't have the information for some reason, then we compute
    /// a name from the SKU and the last couple of bytes from the device id,
    /// similar to the device name that would show up in a BLE scan, or
    /// the default name for the device if not otherwise configured in the
    /// Govee App.
    pub fn name(&self) -> String {
        if let Some(name) = self.govee_name() {
            return name.to_string();
        }
        self.computed_name()
    }

    /// Returns the name defined for the device in the Govee App
    pub fn govee_name(&self) -> Option<&str> {
        if let Some(info) = &self.http_device_info {
            return Some(&info.device_name);
        }
        // The platform API doesn't return every device (eg: offline or shared
        // devices), but the undoc API device list still carries the app-defined
        // name, so prefer that over a computed SKU_MAC name.
        if let Some(info) = &self.undoc_device_info {
            return Some(&info.entry.device_name);
        }
        None
    }

    pub fn room_name(&self) -> Option<&str> {
        if let Some(info) = &self.undoc_device_info {
            return info.room_name.as_deref();
        }
        None
    }

    /// compute a name from the SKU and the last couple of bytes from the
    /// device id, similar to the device name that would show up in a BLE
    /// scan, or the default name for the device if not otherwise configured
    /// in the Govee App.
    pub fn computed_name(&self) -> String {
        // The id is usually "XX:XX:XX:XX:XX:XX:XX:XX" but some devices
        // report it without colons, and in lowercase.  Normalize it.
        let mut id = String::new();
        for c in self.id.chars() {
            if c == ':' {
                continue;
            }
            id.push(c.to_ascii_uppercase());
        }

        format!("{}_{}", self.sku, &id[id.len().saturating_sub(4)..])
    }

    pub fn preferred_poll_interval(&self) -> chrono::Duration {
        match self.device_type() {
            // If the kettle is on, read its temperature more frequently
            DeviceType::Kettle => {
                if self.device_state().map(|s| s.on).unwrap_or(false) {
                    chrono::Duration::seconds(60)
                } else {
                    *POLL_INTERVAL
                }
            }
            _ => *POLL_INTERVAL,
        }
    }

    pub fn ip_addr(&self) -> Option<IpAddr> {
        self.lan_device.as_ref().map(|device| device.ip)
    }

    pub fn set_last_polled(&mut self) {
        self.last_polled.replace(Utc::now());
    }

    pub fn set_nightlight_state(&mut self, params: NotifyHumidifierNightlightParams) {
        self.nightlight_state.replace(params);
    }

    pub fn set_target_humidity(&mut self, percent: u8) {
        self.target_humidity_percent.replace(percent);
    }

    pub fn set_humidifier_work_mode_and_param(&mut self, mode: u8, param: u8) {
        self.humidifier_work_mode.replace(mode);
        self.humidifier_param_by_mode.insert(mode, param);
    }

    /// The held aurora+laser state, defaulting to all-zero if we have neither
    /// decoded a status frame nor sent a write yet. Callers mutate one field of
    /// the returned value and pass it to a write, which re-sends the full frame.
    pub fn aurora_laser_state(&self) -> SetAuroraLaser {
        self.aurora_laser_state.clone().unwrap_or_default()
    }

    /// Replace the held aurora+laser state. Called after a write (so the next
    /// single-field change starts from what we just sent) and by the status
    /// refiners below.
    pub fn set_aurora_laser_state(&mut self, state: SetAuroraLaser) {
        self.aurora_laser_state.replace(state);
    }

    /// Refine the held aurora fields from an aa 11 status frame. Updates only the
    /// aurora on/speed fields the frame carries; laser fields and the color
    /// arrays are left as held (seeded from our writes), since aa 11 doesn't
    /// carry them.
    pub fn refine_aurora_from_status(&mut self, aurora: NotifyAurora) {
        let mut state = self.aurora_laser_state();
        state.aurora_on = aurora.on;
        state.aurora_effect_speed = aurora.speed;
        self.set_aurora_laser_state(state);
    }

    /// Refine the held laser on/off from an aa 34 status frame. Only the on/off
    /// bit is interpreted (see `NotifyLaser::is_on`) and stored in `laser_sub_on`,
    /// the byte that tracked the laser toggle in the live write blob. The laser
    /// value bytes (brightness/swim/flicker) use a packing we haven't pinned, so
    /// they stay as held (seeded from our writes) rather than guessing a mapping.
    pub fn refine_laser_from_status(&mut self, laser: NotifyLaser) {
        let mut state = self.aurora_laser_state();
        state.laser_on = laser.is_on();
        self.set_aurora_laser_state(state);
    }

    /// The held auto-off state, defaulting to disabled if we haven't sent one yet.
    pub fn auto_off_state(&self) -> SetAutoOff {
        self.auto_off_state.unwrap_or_default()
    }

    /// Replace the held auto-off state, called after an auto-off write so the
    /// next single-field change starts from what we just sent.
    pub fn set_auto_off_state(&mut self, state: SetAutoOff) {
        self.auto_off_state.replace(state);
    }

    /// Mark the aurora/laser state as seeded from common-datas, so the one-shot
    /// seed doesn't run again.
    pub fn mark_aurora_laser_seeded(&mut self) {
        self.aurora_laser_seeded = true;
    }

    /// Record a just-written settings-toggle value, so HA can show it (the device
    /// never reports these back). Returns true if `instance` is a settings toggle.
    pub fn record_projector_setting(&mut self, instance: &str, on: bool) -> bool {
        self.projector_settings.record(instance, on)
    }

    /// Update the LAN device information
    pub fn set_lan_device(&mut self, device: LanDevice) {
        self.lan_device.replace(device);
        self.last_lan_device_update.replace(Utc::now());
    }

    /// Update the LAN device status information
    pub fn set_lan_device_status(&mut self, status: LanDeviceStatus) -> bool {
        let changed = self
            .lan_device_status
            .as_ref()
            .map(|prior| *prior != status)
            .unwrap_or(true);
        self.lan_device_status.replace(status);
        self.last_lan_device_status_update.replace(Utc::now());
        self.clear_scene_if_color_changed();
        changed
    }

    pub fn set_iot_device_status(&mut self, status: LanDeviceStatus) {
        self.iot_device_status.replace(status);
        self.last_iot_device_status_update.replace(Utc::now());
        self.clear_scene_if_color_changed();
    }

    /// Number of independently switched outlets for a multi-outlet socket,
    /// or None if this device isn't one
    pub fn socket_outlet_count(&self) -> Option<u8> {
        self.resolve_quirk().and_then(|q| q.socket_outlet_count)
    }

    pub fn set_socket_outlet_bits(&mut self, bits: u8) {
        self.socket_outlet_bits.replace(bits);
    }

    /// State of a single outlet on a multi-outlet socket. Outlet `index`
    /// occupies bit `index` of the reported onOff value.
    pub fn socket_outlet_state(&self, index: u8) -> Option<bool> {
        self.socket_outlet_bits.map(|bits| bits & (1 << index) != 0)
    }

    /// The device's BLE MAC, from the undoc API's per-device settings. None if the
    /// device has no BLE address (e.g. wifi-only devices). Used to find and connect
    /// the peripheral for direct BLE control.
    pub fn ble_address(&self) -> Option<&str> {
        self.undoc_device_info
            .as_ref()?
            .entry
            .device_ext
            .device_settings
            .address
            .as_deref()
    }

    /// The user-assigned name for outlet `index` of a multi-outlet socket, as
    /// configured in the Govee app. Comes from the undoc API's per-device
    /// `subDevices` metadata (`{"sub_0": {"name": "..."}}`), where `sub_<index>`
    /// lines up with bit `index` of the onOff value. None if unavailable.
    pub fn socket_outlet_name(&self, index: u8) -> Option<String> {
        self.undoc_device_info
            .as_ref()?
            .entry
            .device_ext
            .device_settings
            .sub_devices
            .as_ref()?
            .get(format!("sub_{index}"))?
            .get("name")?
            .as_str()
            .map(|s| s.to_string())
    }

    pub fn set_http_device_info(&mut self, mut info: HttpDeviceInfo) {
        // Augment the platform API's capability list with controls it doesn't
        // report but that we drive over IoT (eg: the H6093 projector's settings).
        // The incoming info is fresh from the API each poll and never carries
        // these, so add any that aren't already present by instance.
        for cap in GoveeUndocumentedApi::synthesize_h6093_capabilities(&info.sku) {
            if !info.capabilities.iter().any(|c| c.instance == cap.instance) {
                info.capabilities.push(cap);
            }
        }
        self.http_device_info.replace(info);
        self.last_http_device_update.replace(Utc::now());
    }

    pub fn set_http_device_state(&mut self, state: HttpDeviceState) {
        self.http_device_state.replace(state);
        self.last_http_device_state_update.replace(Utc::now());
        self.clear_scene_if_color_changed();
    }

    pub fn set_undoc_device_info(
        &mut self,
        entry: govee_api::undoc_api::DeviceEntry,
        room_name: Option<&str>,
    ) {
        self.undoc_device_info.replace(UndocDeviceInfo {
            entry,
            room_name: room_name.map(|s| s.to_string()),
        });
        self.last_undoc_device_info_update.replace(Utc::now());
        self.clear_scene_if_color_changed();
    }

    pub fn compute_iot_device_state(&self) -> Option<DeviceState> {
        let updated = self.last_iot_device_status_update?;
        let status = self.iot_device_status.as_ref()?;

        Some(DeviceState {
            on: status.on,
            light_on: if self.device_type() == DeviceType::Light {
                Some(status.on)
            } else {
                self.nightlight_state.as_ref().map(|s| s.on)
            },
            online: None,
            brightness: status.brightness,
            color: status.color,
            kelvin: status.color_temperature_kelvin,
            scene: self.active_scene.as_ref().map(|info| info.name.to_string()),
            source: "AWS IoT API",
            updated,
        })
    }

    pub fn compute_lan_device_state(&self) -> Option<DeviceState> {
        let updated = self.last_lan_device_status_update?;
        let status = self.lan_device_status.as_ref()?;

        Some(DeviceState {
            on: status.on,
            light_on: Some(status.on), // assumption: LAN API == light
            online: None,
            brightness: status.brightness,
            color: status.color,
            kelvin: status.color_temperature_kelvin,
            scene: self.active_scene.as_ref().map(|info| info.name.to_string()),
            source: "LAN API",
            updated,
        })
    }

    pub fn compute_http_device_state(&self) -> Option<DeviceState> {
        let updated = self.last_http_device_state_update?;
        let state = self.http_device_state.as_ref()?;

        let mut online = None;
        let mut on = false;
        let mut light_on = None;
        let mut brightness = 0;
        let mut color = DeviceColor::default();
        let mut kelvin = 0;

        #[derive(serde::Deserialize)]
        struct IntegerValueState {
            value: u32,
        }
        #[derive(serde::Deserialize)]
        struct BoolValueState {
            value: bool,
        }

        let light_instance = self.get_light_power_toggle_instance_name();

        for cap in &state.capabilities {
            if let Ok(value) = serde_json::from_value::<IntegerValueState>(cap.state.clone()) {
                if light_instance
                    .map(|inst| inst == cap.instance.as_str())
                    .unwrap_or(false)
                {
                    light_on.replace(value.value != 0);
                }

                match cap.instance.as_str() {
                    "powerSwitch" => {
                        on = value.value != 0;
                    }
                    "colorRgb" => {
                        color = DeviceColor {
                            r: ((value.value >> 16) & 0xff) as u8,
                            g: ((value.value >> 8) & 0xff) as u8,
                            b: (value.value & 0xff) as u8,
                        };
                    }
                    "brightness" => {
                        brightness = value.value as u8;
                    }
                    "colorTemperatureK" => {
                        kelvin = value.value;
                    }
                    _ => {}
                }
            } else if cap.instance == "online"
                && let Ok(value) = serde_json::from_value::<BoolValueState>(cap.state.clone())
            {
                online.replace(value.value);
            }
        }

        Some(DeviceState {
            on,
            light_on,
            online,
            brightness,
            color,
            kelvin,
            scene: self.active_scene.as_ref().map(|info| info.name.to_string()),
            source: "PLATFORM API",
            updated,
        })
    }

    /// Returns the most recently received state information
    pub fn device_state(&self) -> Option<DeviceState> {
        let mut candidates = vec![];

        if let Some(state) = self.compute_lan_device_state() {
            candidates.push(state);
        }
        if let Some(state) = self.compute_http_device_state() {
            candidates.push(state);
        }
        if let Some(state) = self.compute_iot_device_state() {
            candidates.push(state);
        }

        candidates.sort_by_key(|state| state.updated);

        candidates.pop()
    }

    /// The online flag the undoc device list reports for this device. This is
    /// the only reachability signal we get for shared devices, which the
    /// platform API doesn't return and which we can't poll. It reflects the
    /// cloud's last-known reachability and refreshes when the device list is
    /// re-fetched, so it can lag (eg: a device reads false at rest until
    /// something wakes it).
    pub fn undoc_reported_online(&self) -> Option<bool> {
        self.undoc_device_info
            .as_ref()?
            .entry
            .device_ext
            .last_device_data
            .online
    }

    /// Reachability of the device. This is the single source of truth for both
    /// the Status diagnostic sensor text and the per-device MQTT availability
    /// topic, so the two always agree.
    ///
    /// The platform API answers from the cloud whether or not the device itself
    /// is reachable, and its response carries an explicit `online` flag for the
    /// device (DeviceState::online). When the freshest state has that flag, it
    /// is authoritative: a device the cloud reports offline must read offline
    /// even though we just fetched its (stale) state successfully. Otherwise we
    /// would only ever fetch cached settings and never notice the device left.
    ///
    /// LAN and IoT replies carry no online flag, but a reply only arrives when
    /// the device answered (LAN) or is connected to AWS IoT (IoT), so the
    /// arrival itself is the reachability signal. For those we fall back to how
    /// recently we last heard anything: past the availability timeout means the
    /// device stopped answering and is treated as gone. The IoT poll cadence is
    /// tied to half that timeout (see iot_resend_interval), so a live device is
    /// re-probed before the window elapses.
    ///
    /// With no polled state at all (eg: shared devices the platform API doesn't
    /// return and we can't poll), use the undoc device list's online flag.
    pub fn availability_status(&self) -> Reachability {
        if let Some(state) = self.device_state() {
            if let Some(online) = state.online {
                return if online {
                    Reachability::Available
                } else {
                    Reachability::Missing
                };
            }

            return if Utc::now() - state.updated > availability_timeout() {
                Reachability::Missing
            } else {
                Reachability::Available
            };
        }

        // No polled state. Fall back to the undoc device list's online flag,
        // which is what shared devices report.
        match self.undoc_reported_online() {
            Some(true) => Reachability::Available,
            Some(false) => Reachability::Missing,
            None => Reachability::Unknown,
        }
    }

    /// Records the active scene name
    pub fn set_active_scene(&mut self, scene: Option<&str>) {
        match scene {
            None => {
                self.active_scene.take();
            }
            Some(scene) => {
                let (color, kelvin) = self
                    .device_state()
                    .map(|s| (s.color, s.kelvin))
                    .unwrap_or_default();
                self.active_scene.replace(ActiveSceneInfo {
                    name: scene.to_string(),
                    color,
                    kelvin,
                });
            }
        }
    }

    /// The user's chosen music sensitivity, defaulting to 100.
    pub fn music_sensitivity(&self) -> u8 {
        self.music_sensitivity.unwrap_or(100)
    }

    pub fn set_music_sensitivity(&mut self, value: u8) {
        self.music_sensitivity = Some(value.min(100));
    }

    /// The user's chosen music auto-color, defaulting to on.
    pub fn music_auto_color(&self) -> bool {
        self.music_auto_color.unwrap_or(true)
    }

    pub fn set_music_auto_color(&mut self, value: bool) {
        self.music_auto_color = Some(value);
    }

    pub fn clear_scene_if_color_changed(&mut self) {
        if let Some(info) = &self.active_scene {
            let current = self
                .device_state()
                .map(|s| (s.color, s.kelvin))
                .unwrap_or_default();
            let scene_state = (info.color, info.kelvin);
            if current != scene_state {
                log::info!(
                    "Clearing reported scene because current {current:?} != {scene_state:?}"
                );
                self.active_scene.take();
            }
        }
    }

    pub fn device_type(&self) -> DeviceType {
        if let Some(info) = &self.http_device_info {
            info.device_type.clone()
        } else if let Some(q) = resolve_quirk(&self.sku) {
            q.device_type.clone()
        } else {
            DeviceType::Light
        }
    }

    /// Indicate whether we require the platform API data in order
    /// to correctly report the device
    pub fn needs_platform_poll(&self) -> bool {
        if !self.iot_api_supported() {
            return true;
        }

        let device_type = self.device_type();
        match (device_type, self.sku.as_str()) {
            (_, "H7160") => false,
            (DeviceType::Humidifier, _) => true,
            (DeviceType::Light, _) => false,
            (DeviceType::Kettle, _) => true,
            _ => true,
        }
    }

    /// Whether we have any signal for this device's reachability: a transport we
    /// can poll, or the undoc device list's online flag (the only signal shared
    /// devices give us). A device with no signal at all must not get a
    /// per-device availability topic, or it would be pinned offline forever; it
    /// falls back to the bridge availability instead. See [`availability_status`].
    pub fn has_reachability_signal(&self) -> bool {
        self.pollable_via_lan()
            || self.pollable_via_iot()
            || self.http_device_info.is_some()
            || self.undoc_reported_online().is_some()
    }

    pub fn pollable_via_lan(&self) -> bool {
        self.lan_device.is_some()
    }

    pub fn pollable_via_iot(&self) -> bool {
        if !self.iot_api_supported() {
            return false;
        }
        let device_type = self.device_type();
        matches!(
            (device_type, self.sku.as_str()),
            (_, "H7160") | (DeviceType::Light, _)
        )
    }

    pub fn avoid_platform_api(&self) -> bool {
        if let Some(quirk) = self.resolve_quirk() {
            if quirk.avoid_platform_api {
                return true;
            }
            if self.lan_device.is_some()
                && !self
                    .http_device_info
                    .as_ref()
                    .map(|info| info.supports_rgb())
                    .unwrap_or(false)
            {
                // Conflicting information:
                // Platform API says that this device isn't
                // a light, but the LAN API support suggests
                // that it is a light!
                // Therefore we will not trust the Platform API
                return true;
            }
        }
        false
    }

    pub fn resolve_quirk(&self) -> Option<Quirk> {
        match resolve_quirk(&self.sku) {
            Some(q) => Some(q.clone()),
            None => {
                // It's an unknown device, but since it showed up via LAN disco,
                // we can assume that it is a light
                if self.lan_device.is_some() {
                    Some(Quirk::light(Cow::Owned(self.sku.to_string()), BULB).with_lan_api())
                } else {
                    None
                }
            }
        }
    }

    pub fn get_capability_by_instance(&self, instance: &str) -> Option<&DeviceCapability> {
        self.http_device_info
            .as_ref()
            .and_then(|info| info.capability_by_instance(instance))
    }

    pub fn get_state_capability_by_instance(
        &self,
        instance: &str,
    ) -> Option<DeviceCapabilityState> {
        if let Some(cap) = self
            .http_device_state
            .as_ref()
            .and_then(|info| info.capability_by_instance(instance))
        {
            return Some(cap.clone());
        }
        // Fall back to synthesized state for IoT-only controls (eg: the H6093
        // aurora/laser, auto-off, and settings-toggle entities) whose value we
        // hold ourselves rather than getting from the platform API.
        let (kind, state) = govee_api::ble::projector_state_value(
            instance,
            &self.aurora_laser_state(),
            &self.auto_off_state(),
            &self.projector_settings,
        )?;
        Some(DeviceCapabilityState {
            kind,
            instance: instance.to_string(),
            state,
        })
    }

    pub fn get_light_power_toggle_instance_name(&self) -> Option<&'static str> {
        match self.device_type() {
            DeviceType::Light => Some("powerSwitch"),
            _ => {
                // If the device's primary function is not a light,
                // then we need to avoid powering on its other function
                // here.  If it has a nightlight capability, that is
                // probably what we are controlling.
                // We may need to expand this to other power toggles
                // in the future.
                if self
                    .get_capability_by_instance("nightlightToggle")
                    .is_some()
                {
                    Some("nightlightToggle")
                } else {
                    None
                }
            }
        }
    }

    pub fn get_color_temperature_range(&self) -> Option<(u32, u32)> {
        if let Some(quirk) = self.resolve_quirk() {
            return quirk.color_temp_range;
        }

        if self.lan_device.is_some() {
            // LAN API support suggests that it is a light
            return Some((2000, 9000));
        }

        self.http_device_info
            .as_ref()
            .and_then(|info| info.get_color_temperature_range())
    }

    pub fn supports_brightness(&self) -> bool {
        if let Some(quirk) = self.resolve_quirk() {
            return quirk.supports_brightness;
        }

        if self.lan_device.is_some() {
            // LAN API support suggests that it is a light
            return true;
        }

        self.http_device_info
            .as_ref()
            .map(|info| info.supports_brightness())
            .unwrap_or(false)
    }

    pub fn iot_api_supported(&self) -> bool {
        if let Some(quirk) = self.resolve_quirk() {
            return quirk.iot_api_supported;
        }

        false
    }

    pub fn supports_rgb(&self) -> bool {
        if let Some(quirk) = self.resolve_quirk() {
            return quirk.supports_rgb;
        }

        if self.lan_device.is_some() {
            // LAN API support suggests that it is a light
            return true;
        }

        self.http_device_info
            .as_ref()
            .map(|info| info.supports_rgb())
            .unwrap_or(false)
    }

    pub fn is_ble_only_device(&self) -> Option<bool> {
        if let Some(quirk) = self.resolve_quirk() {
            return Some(quirk.ble_only);
        }

        if self.http_device_info.is_some() {
            // truly BLE-only devices are not returned via the Platform API,
            // unless we have a quirk to say otherwise
            return Some(false);
        }

        self.undoc_device_info
            .as_ref()
            .map(|info| info.entry.device_ext.device_settings.wifi_name.is_none())
    }

    pub fn is_controllable(&self) -> bool {
        !matches!(self.is_ble_only_device(), Some(true))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn name_compute() {
        let device = Device::new("H6000", "AA:BB:CC:DD:EE:FF:42:2A");
        assert_eq!(device.name(), "H6000_422A");

        let device = Device::new("H6127", "cef142b0b354995f");
        assert_eq!(device.name(), "H6127_995F");

        let device = Device::new("H6127", "ce");
        assert_eq!(device.name(), "H6127_CE");
    }

    /// A platform-API state with an explicit online capability set to `value`,
    /// as the cloud returns it. The cloud answers whether or not the device is
    /// reachable, so this flag, not how recently we fetched, is authoritative.
    fn http_state_with_online(sku: &str, id: &str, value: bool) -> HttpDeviceState {
        serde_json::from_value(serde_json::json!({
            "sku": sku,
            "device": id,
            "capabilities": [{
                "type": "devices.capabilities.online",
                "instance": "online",
                "state": {"value": value}
            }]
        }))
        .expect("valid HttpDeviceState")
    }

    #[test]
    fn platform_online_flag_is_authoritative_over_freshness() {
        // The cloud says offline. Even though we fetched the state just now, the
        // device must read Missing: the platform API answers from the cloud for
        // a device that itself is unreachable (eg: its wifi dropped while our
        // bridge kept cloud connectivity).
        let mut device = Device::new("H6109", "AA:BB:CC:DD:EE:FF:11:22");
        device.set_http_device_state(http_state_with_online("H6109", &device.id, false));
        assert_eq!(device.availability_status(), Reachability::Missing);

        // Same fetch timing, cloud says online: Available.
        let mut device = Device::new("H6109", "AA:BB:CC:DD:EE:FF:11:22");
        device.set_http_device_state(http_state_with_online("H6109", &device.id, true));
        assert_eq!(device.availability_status(), Reachability::Available);
    }

    #[test]
    fn iot_state_uses_freshness_when_no_online_flag() {
        // IoT and LAN replies carry no online flag; a reply arriving at all is
        // the reachability signal. A fresh reply reads Available.
        let mut device = Device::new("H6109", "AA:BB:CC:DD:EE:FF:33:44");
        device.set_iot_device_status(govee_api::lan_api::DeviceStatus::default());
        assert_eq!(device.availability_status(), Reachability::Available);

        // Just inside the availability window: still Available.
        device.last_iot_device_status_update =
            Some(Utc::now() - (availability_timeout() - chrono::Duration::seconds(10)));
        assert_eq!(device.availability_status(), Reachability::Available);

        // Past the availability timeout: Missing. Availability is driven by the
        // timeout, not POLL_INTERVAL (the 15min state poll), so a slow poll can't
        // keep a dead device marked Available.
        device.last_iot_device_status_update =
            Some(Utc::now() - (availability_timeout() + chrono::Duration::seconds(10)));
        assert_eq!(device.availability_status(), Reachability::Missing);
    }

    #[test]
    fn no_state_falls_back_to_undoc_online_flag() {
        let device = Device::new("H6109", "AA:BB:CC:DD:EE:FF:55:66");
        assert_eq!(device.availability_status(), Reachability::Unknown);
    }
}
