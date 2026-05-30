use crate::commands::serve::{POLL_INTERVAL, availability_timeout};
use crate::service::quirks::{BULB, Quirk, resolve_quirk};
use crate::service::state::{ClientAvail, Transport};
use chrono::{DateTime, Utc};
use govee_api::ble::{
    GoveeBlePacket, NotifyAurora, NotifyHumidifierNightlightParams, NotifyLaser, ProjectorSettings,
    SetAuroraLaser, SetAutoOff,
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

    /// Held H5082 countdown state, keyed by `(outlet_wire, kind_wire)`. The
    /// device emits one `aa b0` per slot on every status broadcast (four slots
    /// total: two outlets x two kinds), so the map is rebuilt as broadcasts
    /// arrive. Empty until the first broadcast.
    pub h5082_countdowns: HashMap<(u8, u8), govee_api::ble::socket::NotifyCountdown>,

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

    /// Which transport the displayed state came from
    pub source: Transport,
    pub updated: DateTime<Utc>,
}

/// Wire shape used by the http /api/devices list and the /ws state-change
/// stream. A clean projection of Device so internal fields (http_device_info,
/// raw quirks, transport handles) don't leak over the wire.
#[derive(Serialize, Clone, Debug)]
pub struct DeviceItem {
    pub sku: String,
    pub id: String,
    pub name: String,
    pub room: Option<String>,
    pub ip: Option<IpAddr>,
    pub state: Option<DeviceState>,
    /// Static control surface for this SKU, derived from the matched quirk.
    /// Carried in the snapshot so the UI can decide which controls to render
    /// without a separate fetch per device.
    pub capabilities: DeviceCapabilities,
    /// Per-outlet state for multi-outlet sockets. Indexed by outlet number;
    /// `outlets[i]` is true when outlet `i` is on. None when the device isn't
    /// a multi-outlet socket or no IoT status with the bits has been received
    /// yet. Decoded from `socket_outlet_bits`, not synthesized per source.
    ///
    /// Index `i` is wire bit `i`, which is the Govee app's `sub_i` (see
    /// `socket_outlet_name`). The user-facing outlet number the Govee app
    /// shows is whatever the user named `sub_i.name`, and is independent of
    /// `i`: a device whose Govee app says "Outlet 1" may store that name in
    /// `sub_0` or in `sub_1`, and the physical-side mapping (left/right) is
    /// per-SKU. HA's entity labels mirror the `sub_N` names so the Govee app
    /// and HA agree; consumers that label by raw index (e.g. the Web UI's
    /// `#0`, `#1`) won't match the Govee app's numbering unless `sub_N.name`
    /// happens to be `"Outlet N+1"`.
    pub outlets: Option<Vec<bool>>,
    /// True when the device is shared into the account rather than owned.
    /// Surfaces in the UI so users can tell at a glance that control will go
    /// via the REST relay (and that platform-API state polls aren't available
    /// for it).
    pub shared: bool,
}

/// What controls make sense for this device. Used by the UI to decide which
/// sliders, pickers, and toggles to render. All fields are derived from the
/// matched quirk; a device with no quirk reports a power-only baseline.
#[derive(Serialize, Clone, Debug)]
pub struct DeviceCapabilities {
    /// Every device exposes power. Constant true, kept as a field so the
    /// shape is regular if a future device type omits power.
    pub power: bool,
    pub brightness: bool,
    pub rgb: bool,
    /// `[min, max]` kelvin range when the device supports color temperature.
    pub color_temp_kelvin: Option<[u32; 2]>,
    /// Number of independently switched outlets for multi-outlet sockets
    /// (eg H5082). None for everything else.
    pub socket_outlets: Option<u8>,
}

impl DeviceCapabilities {
    pub fn from_device(d: &Device) -> Self {
        let quirk = d.resolve_quirk();
        Self {
            power: true,
            brightness: quirk
                .as_ref()
                .and_then(|q| q.supports_brightness)
                .unwrap_or(false),
            rgb: quirk.as_ref().and_then(|q| q.supports_rgb).unwrap_or(false),
            color_temp_kelvin: quirk
                .as_ref()
                .and_then(|q| q.color_temp_range)
                .map(|(min, max)| [min, max]),
            socket_outlets: d.socket_outlet_count(),
        }
    }
}

impl DeviceItem {
    pub fn snapshot(d: &Device) -> Self {
        // for multi-outlet sockets, expand the bitfield into a per-outlet vec.
        // None means we don't know yet (no IoT status received), as distinct
        // from a known-all-off state which would be Some(vec![false; count]).
        let outlets = d.socket_outlet_count().and_then(|count| {
            (0..count)
                .map(|i| d.socket_outlet_state(i))
                .collect::<Option<Vec<bool>>>()
        });
        Self {
            sku: d.sku.clone(),
            id: d.id.clone(),
            name: d.name(),
            room: d.room_name().map(|r| r.to_string()),
            ip: d.ip_addr(),
            state: d.device_state(),
            capabilities: DeviceCapabilities::from_device(d),
            outlets,
            shared: d.is_shared(),
        }
    }
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

    /// Fold a decoded BLE status packet into held state, returning true when it
    /// updated something the caller should publish. Shared by the IoT status
    /// subscriber (which receives these frames base64-wrapped in op.command) and
    /// the direct-BLE reader (which receives them as aa notifications): both see
    /// the same status frames, so the dispatch lives here once. Command echoes
    /// and frames we can't decode return false.
    pub fn apply_ble_status(&mut self, decoded: &GoveeBlePacket) -> bool {
        match decoded {
            GoveeBlePacket::NotifyHumidifierNightlight(nl) => {
                self.set_nightlight_state(*nl);
                true
            }
            GoveeBlePacket::NotifyHumidifierAutoMode(m) => {
                self.set_target_humidity(m.target_humidity.as_percent());
                true
            }
            GoveeBlePacket::NotifyHumidifierMode(m) => {
                self.set_humidifier_work_mode_and_param(m.mode, m.param);
                true
            }
            GoveeBlePacket::NotifyAurora(aurora) => {
                self.refine_aurora_from_status(*aurora);
                true
            }
            GoveeBlePacket::NotifyLaser(laser) => {
                self.refine_laser_from_status(*laser);
                true
            }
            GoveeBlePacket::NotifyCountdown(countdown) => {
                self.record_h5082_countdown(*countdown);
                true
            }
            GoveeBlePacket::NotifyOutletState(s) => self.set_socket_outlet_bits(s.bits),
            _ => false,
        }
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

    /// Record one H5082 countdown slot from an `aa b0` read on the IoT status
    /// broadcast. Keyed by the `(outlet_wire, kind_wire)` pair the frame
    /// carries; overwrites any prior value for that slot.
    pub fn record_h5082_countdown(&mut self, c: govee_api::ble::socket::NotifyCountdown) {
        self.h5082_countdowns.insert((c.outlet, c.kind), c);
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

    /// Returns whether the bits changed, so a caller (the BLE reader, which
    /// re-reads outlet state on every keepalive) can skip republishing when the
    /// state is unchanged.
    pub fn set_socket_outlet_bits(&mut self, bits: u8) -> bool {
        let changed = self.socket_outlet_bits != Some(bits);
        self.socket_outlet_bits.replace(bits);
        changed
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

    /// The per-device BLE secret, base64-decoded from the cloud `secret_code`.
    /// The app writes this as the 33 b2 SECRET_KEY_CHECK probe; supported sockets
    /// gate control writes on it. Per-device, not a constant. None when the device
    /// list carried no secret_code or it didn't decode to 8 bytes.
    pub fn ble_secret_code(&self) -> Option<[u8; 8]> {
        let code = self
            .undoc_device_info
            .as_ref()?
            .entry
            .device_ext
            .device_settings
            .secret_code
            .as_ref()?;
        data_encoding::BASE64
            .decode(code.as_bytes())
            .ok()?
            .try_into()
            .ok()
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
        for cap in GoveeUndocumentedApi::synthesize_h5082_capabilities(&info.sku) {
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
            source: Transport::Iot,
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
            source: Transport::Lan,
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
                        // brightness is documented as 0-100 percent; clamp at
                        // the contract so an out-of-spec API response can't
                        // silently truncate via the u32 -> u8 cast.
                        brightness = value.value.min(100) as u8;
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
            source: Transport::Platform,
            updated,
        })
    }

    /// Pick the best state from whichever sources we have.
    ///
    /// Within a freshness window we prefer the real-time sources (LAN > IoT)
    /// over the platform HTTP poll. The platform API lags a real-world state
    /// change by 30-60s (Govee cloud sync), so a platform poll that arrives a
    /// few seconds after a fresh IoT push usually carries pre-change state.
    /// Last-write-wins on raw timestamp would let that stale poll roll the UI
    /// back, which is exactly the visible flap we're trying to avoid.
    ///
    /// Outside the freshness window the polling sources are the only thing we
    /// have, so fall back to the most-recent across all sources rather than
    /// returning None.
    pub fn device_state(&self) -> Option<DeviceState> {
        const FRESHNESS_SECS: i64 = 60;

        let lan = self.compute_lan_device_state();
        let iot = self.compute_iot_device_state();
        let http = self.compute_http_device_state();

        let now = Utc::now();
        let is_fresh = |s: &DeviceState| (now - s.updated).num_seconds() < FRESHNESS_SECS;

        if let Some(s) = &lan
            && is_fresh(s)
        {
            return lan;
        }
        if let Some(s) = &iot
            && is_fresh(s)
        {
            return iot;
        }
        if let Some(s) = &http
            && is_fresh(s)
        {
            return http;
        }

        [lan, http, iot]
            .into_iter()
            .flatten()
            .max_by_key(|s| s.updated)
    }

    /// Whether this device is shared into the account rather than owned by it.
    /// Shared devices come from the undoc device list (the platform API only
    /// returns owned devices), can't be polled, and have to be controlled via
    /// the REST relay carrying the `gas` token rather than direct AWS IoT
    /// publishes. Defaults to false when we have no undoc record for the device.
    pub fn is_shared(&self) -> bool {
        self.undoc_device_info
            .as_ref()
            .map(|info| info.entry.is_shared())
            .unwrap_or(false)
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
    /// The platform API's response carries an explicit `online` flag, sourced
    /// from the Govee cloud's per-device registry. We use it as the
    /// truly-left-the-network signal: if IoT and LAN both go silent past the
    /// availability window, an HTTP poll that returns cached state could
    /// otherwise keep us reporting Available indefinitely. But the cloud's
    /// online flag lags by around a minute, so a transient blip on a still-live
    /// device would flip availability to Missing on its own. To avoid that
    /// flicker, an `online: false` from the cloud is overridden when we
    /// received an IoT or LAN message inside the availability window: a fresh
    /// message arriving from the device is stronger proof of life than a stale
    /// cloud registry.
    ///
    /// LAN and IoT replies carry no online flag, but a reply only arrives when
    /// the device answered (LAN) or is connected to AWS IoT (IoT), so the
    /// arrival itself is the reachability signal. For those we fall back to how
    /// recently we last heard anything: past the availability timeout means the
    /// device stopped answering and is treated as gone. The poll cadence is
    /// tied to half that timeout (see state_refresh_interval), so a live device
    /// is re-probed before the window elapses.
    ///
    /// With no polled state at all (eg: shared devices the platform API doesn't
    /// return and we can't poll), use the undoc device list's online flag.
    pub fn availability_status(&self) -> Reachability {
        if let Some(state) = self.device_state() {
            if let Some(online) = state.online {
                if !online && self.has_recent_local_signal() {
                    return Reachability::Available;
                }
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

    /// True if an IoT or LAN message arrived from this device inside the
    /// availability window. Used to override a stale `online: false` from the
    /// cloud registry: a fresh local message is direct evidence the device is
    /// up, regardless of what the cloud thinks.
    fn has_recent_local_signal(&self) -> bool {
        let cutoff = Utc::now() - availability_timeout();
        let fresh = |ts: Option<DateTime<Utc>>| ts.is_some_and(|t| t > cutoff);
        fresh(self.last_iot_device_status_update) || fresh(self.last_lan_device_status_update)
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
                log::debug!(
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
        // aurora/laser, auto-off, and settings-toggle entities; the H5082
        // countdown remaining-seconds sensors) whose value we hold ourselves
        // rather than getting from the platform API. Families are tried in
        // order; each owns a disjoint instance namespace.
        let (kind, state) = govee_api::ble::projector_state_value(
            instance,
            &self.aurora_laser_state(),
            &self.auto_off_state(),
            &self.projector_settings,
        )
        .or_else(|| govee_api::ble::socket::state_value(instance, &self.h5082_countdowns))?;
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
        // Quirk is a layered override: Some(range) wins, None falls through.
        // There is no way to express "I explicitly know this device has no
        // color temp" today; if that case shows up we'll need a tri-state.
        if let Some(range) = self.resolve_quirk().and_then(|q| q.color_temp_range) {
            return Some(range);
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
        // Quirk is a layered override: Some(_) wins, None falls through to
        // the LAN/platform sources below.
        if let Some(value) = self.resolve_quirk().and_then(|q| q.supports_brightness) {
            return value;
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

    /// Whether a transport can currently carry a command to this device, from
    /// the device's facts and the sampled client availability. This is the one
    /// definition of "is transport T usable"; the control cascade gates each
    /// step on it, and the debug `effective_transports` display reads it, so the
    /// two can't drift. LAN sends go over the device's own UDP socket (no shared
    /// client), so it only needs `lan_device`; the rest need their client up.
    ///
    /// BLE needs an address, an adapter, AND a family that actually has codecs
    /// for the SKU -- a bare address with no codecs (e.g. an unpinned H5083) is
    /// not controllable. IoT needs `iot_api_supported`, the quirk flag every
    /// socket and IoT-driven device sets. `avoid_platform_api` is not checked
    /// here: the generic verbs use the platform API regardless of it, and only
    /// the scene verb treats it as a routing preference.
    pub fn transport_reachable(&self, transport: Transport, avail: &ClientAvail) -> bool {
        match transport {
            Transport::Lan => self.lan_device.is_some(),
            Transport::Ble => {
                avail.ble
                    && self.ble_address().is_some()
                    && govee_api::ble::sku_has_ble_support(&self.sku)
            }
            Transport::Iot => {
                avail.iot && self.undoc_device_info.is_some() && self.iot_api_supported()
            }
            Transport::Platform => avail.platform && self.http_device_info.is_some(),
        }
    }

    pub fn supports_rgb(&self) -> bool {
        // Quirk is a layered override: Some(_) wins, None falls through to
        // the LAN/platform sources below.
        if let Some(value) = self.resolve_quirk().and_then(|q| q.supports_rgb) {
            return value;
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
    /// as the cloud returns it. The cloud is the truly-left-the-network signal
    /// when no other transport is reporting; a fresh IoT or LAN message
    /// overrides it when the cloud's registry is lagging.
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
    fn platform_online_false_reads_missing_without_other_signals() {
        // The cloud says offline and we have no IoT/LAN signal to contradict it.
        // The HTTP fetch returning successfully doesn't prove the device is up,
        // because the platform API answers from the cloud's cache, so trust the
        // online flag and read Missing.
        let mut device = Device::new("H6109", "AA:BB:CC:DD:EE:FF:11:22");
        device.set_http_device_state(http_state_with_online("H6109", &device.id, false));
        assert_eq!(device.availability_status(), Reachability::Missing);

        // Same fetch timing, cloud says online: Available.
        let mut device = Device::new("H6109", "AA:BB:CC:DD:EE:FF:11:22");
        device.set_http_device_state(http_state_with_online("H6109", &device.id, true));
        assert_eq!(device.availability_status(), Reachability::Available);
    }

    #[test]
    fn recent_iot_signal_overrides_cloud_offline_flag() {
        // The cloud registry lags by about a minute. If it says offline but
        // we received an IoT message inside the availability window, the
        // device is provably up and we override to Available. This kills the
        // ~15-minute flicker that the HTTP poll would otherwise cause when
        // the cloud catches a transient blip on a still-live device.
        let mut device = Device::new("H6109", "AA:BB:CC:DD:EE:FF:77:88");
        device.set_http_device_state(http_state_with_online("H6109", &device.id, false));
        device.set_iot_device_status(govee_api::lan_api::DeviceStatus::default());
        assert_eq!(device.availability_status(), Reachability::Available);

        // Once the IoT signal ages past the window with no refresh, the
        // override stops applying and the cloud's verdict wins again.
        device.last_iot_device_status_update =
            Some(Utc::now() - (availability_timeout() + chrono::Duration::seconds(10)));
        assert_eq!(device.availability_status(), Reachability::Missing);
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

    /// Build an HttpDeviceState with a single powerSwitch capability so the
    /// `on` field flows through compute_http_device_state.
    fn http_state_with_power(sku: &str, id: &str, on: bool) -> HttpDeviceState {
        serde_json::from_value(serde_json::json!({
            "sku": sku,
            "device": id,
            "capabilities": [{
                "type": "devices.capabilities.on_off",
                "instance": "powerSwitch",
                "state": {"value": if on { 1 } else { 0 }}
            }]
        }))
        .expect("valid HttpDeviceState")
    }

    #[test]
    fn fresh_iot_beats_slightly_later_platform_poll() {
        // Recreates the visible-flap scenario: user toggles a light on, IoT
        // pushes the new state fast, then the Govee cloud's lagging poll
        // returns a few seconds later carrying pre-change state. Last-write-
        // wins would let the stale poll roll the UI back. The freshness
        // window must prefer the IoT push instead.
        let mut device = Device::new("H6109", "AA:BB:CC:DD:EE:FF:9A:9B");

        // IoT pushed "on" 5 seconds ago.
        device.set_iot_device_status(govee_api::lan_api::DeviceStatus {
            on: true,
            ..Default::default()
        });
        device.last_iot_device_status_update = Some(Utc::now() - chrono::Duration::seconds(5));

        // Platform poll landed just now reporting the pre-change "off".
        device.set_http_device_state(http_state_with_power("H6109", &device.id, false));

        let state = device.device_state().expect("a state");
        assert_eq!(state.source, Transport::Iot);
        assert!(state.on);
    }

    #[test]
    fn stale_iot_falls_back_to_recent_platform_poll() {
        // The complement: when IoT hasn't sent anything in minutes, a fresh
        // platform poll is the trustworthy source. Falling back to last-
        // write-wins here keeps a long-dead IoT source from pinning stale data.
        let mut device = Device::new("H6109", "AA:BB:CC:DD:EE:FF:9C:9D");

        device.set_iot_device_status(govee_api::lan_api::DeviceStatus {
            on: true,
            ..Default::default()
        });
        device.last_iot_device_status_update = Some(Utc::now() - chrono::Duration::seconds(600));

        device.set_http_device_state(http_state_with_power("H6109", &device.id, false));

        let state = device.device_state().expect("a state");
        assert_eq!(state.source, Transport::Platform);
        assert!(!state.on);
    }
}
