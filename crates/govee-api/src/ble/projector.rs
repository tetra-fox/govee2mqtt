//! BLE command structs for the H6093 "Stars" aurora/laser projector and the
//! codecs that encode and decode them. The frame layouts were captured from the
//! app's MQTT control stream and the device's status echoes, and decoded by
//! correlating wire bytes against the app's per-field state writes.
//!
//! The projector has two independent light layers (aurora/nebula and
//! laser/stars) plus a settings panel. Each control is a 20-byte frame: a 0x33
//! write mirrors the 0xaa status read for the same opcode.

use super::codec::{Base64HexBytes, PacketCodec, finish};
use crate::packet;
use serde::Serialize;
use serde_json::Value as JsonValue;

/// Instance names for the H6093's IoT-encoded capabilities. These are the keys
/// that tie a synthesized `DeviceCapability` (what HA enumerates) to its frame
/// encoder (what the wire needs), so the generic IoT dispatch can route a
/// command by instance name without any per-SKU branching in the dispatch code.
pub mod instance {
    // Standalone toggles (each its own 33 30 frame).
    pub const PAIRING_STATUS: &str = "pairingStatus";
    pub const PAIRING_SOUND: &str = "pairingSound";
    pub const SILENT_POWER_UP: &str = "silentPowerUp";
    pub const DREAMVIEW_LASER: &str = "dreamViewLaser";

    // Aurora + stars controls. These all share the one aurora+laser write blob,
    // so changing one reads the held state, mutates this field, re-sends the
    // whole frame (see `apply_blob_field`). Names use the app's labels: the
    // laser layer is "stars", its speeds are "orbit" and "flashing".
    pub const AURORA_ON: &str = "auroraOn";
    pub const AURORA_BRIGHTNESS: &str = "auroraBrightness";
    pub const AURORA_EFFECT: &str = "auroraEffect";
    pub const AURORA_EFFECT_SPEED: &str = "auroraEffectSpeed";
    pub const AURORA_FLOW: &str = "auroraFlow";
    /// The app's "Aurora High" toggle is really the basic/advanced color-mode
    /// selector; exposed as a select rather than a switch.
    pub const AURORA_COLOR_MODE: &str = "auroraColorMode";
    pub const STARS_ON: &str = "starsOn";
    pub const STARS_BRIGHTNESS: &str = "starsBrightness";
    pub const ORBIT_ON: &str = "orbitOn";
    pub const ORBIT_SPEED: &str = "orbitSpeed";
    pub const FLASHING_ON: &str = "flashingOn";
    pub const FLASHING_SPEED: &str = "flashingSpeed";

    // Auto-off cluster. Enable, the "stop playing sound" sub-option, and the
    // timeout minutes share the one 33 30 05 frame (see `apply_auto_off_field`).
    pub const AUTO_OFF_ENABLE: &str = "autoOffEnable";
    pub const AUTO_OFF_STOP_SOUND: &str = "autoOffStopSound";
    pub const AUTO_OFF_MINUTES: &str = "autoOffMinutes";
}

/// Encode a control command for an IoT-only (ptReal-framed) capability into the
/// base64 frames `IotClient::send_real` expects. Returns `None` when this
/// SKU+instance is not one we frame-encode, so the caller falls back to the
/// platform API. The boolean toggles read their on/off from the HA payload.
///
/// This is the single point that maps (sku, instance, value) -> frames; adding a
/// new IoT-only control is a new arm here plus its codec, with no change to the
/// dispatch path.
pub fn encode_capability(
    sku: &str,
    instance: &str,
    value: &JsonValue,
) -> Option<anyhow::Result<Vec<String>>> {
    if sku != "H6093" {
        return None;
    }

    let on = || value.as_bool().or_else(|| value.as_i64().map(|v| v != 0));

    let frames = match instance {
        instance::PAIRING_STATUS => encode_toggle(sku, SetPairingStatus { on: on()? }),
        instance::PAIRING_SOUND => encode_toggle(sku, SetPairingSound { on: on()? }),
        instance::SILENT_POWER_UP => encode_toggle(sku, SetSilentPowerUp { on: on()? }),
        instance::DREAMVIEW_LASER => encode_toggle(sku, SetDreamViewLaser { on: on()? }),
        _ => return None,
    };
    Some(frames)
}

fn encode_toggle<T: 'static>(sku: &str, value: T) -> anyhow::Result<Vec<String>> {
    Ok(Base64HexBytes::encode_for_sku(sku, &value)?.base64())
}

/// The common-datas record that seeds this SKU's held control state, as
/// `(bizType, bizKey)`, or None if the SKU isn't seeded from common-datas.
/// common-datas is the app's generic per-device UI-state store; only SKUs whose
/// synthesized entities have no platform/IoT status source need seeding from it.
/// The H6093 keeps its full aurora/laser state under bizType 3, key `<sku>_<id>`
/// (parsed by `SetAuroraLaser::from_common_datas`).
pub fn common_datas_seed(sku: &str, device_id: &str) -> Option<(i32, String)> {
    match sku {
        "H6093" => Some((3, format!("{sku}_{device_id}"))),
        _ => None,
    }
}

/// Apply a single aurora/stars control change onto the held aurora+laser state.
/// Returns true if `instance` is one of the shared-blob fields (so the caller
/// re-encodes and sends the whole `SetAuroraLaser`); false if it isn't, so the
/// caller can try the standalone encoders or fall through. The (instance ->
/// field) mapping is the only H6093-specific knowledge; the read-mutate-resend
/// cycle lives generically in the transport layer.
pub fn apply_blob_field(instance: &str, value: &JsonValue, state: &mut SetAuroraLaser) -> bool {
    let b = || value.as_bool().or_else(|| value.as_i64().map(|v| v != 0));
    let n = || value.as_i64().map(|v| v.clamp(0, 255) as u8);
    match instance {
        instance::AURORA_ON => state.aurora_on = b().unwrap_or(state.aurora_on),
        instance::AURORA_BRIGHTNESS => {
            state.aurora_brightness = n().unwrap_or(state.aurora_brightness)
        }
        instance::AURORA_EFFECT => {
            state.aurora_effect_code = n().unwrap_or(state.aurora_effect_code)
        }
        instance::AURORA_EFFECT_SPEED => {
            state.aurora_effect_speed = n().unwrap_or(state.aurora_effect_speed)
        }
        instance::AURORA_FLOW => state.aurora_flow = n().unwrap_or(state.aurora_flow),
        instance::AURORA_COLOR_MODE => {
            // Select value is "Basic" / "Advanced". Anything else leaves it.
            match value.as_str() {
                Some("Advanced") => state.color_mode = AuroraColorMode::Advanced,
                Some("Basic") => state.color_mode = AuroraColorMode::Basic,
                _ => {}
            }
        }
        instance::STARS_ON => state.laser_on = b().unwrap_or(state.laser_on),
        instance::STARS_BRIGHTNESS => {
            state.laser_brightness = n().unwrap_or(state.laser_brightness)
        }
        instance::ORBIT_ON => state.swim_on = b().unwrap_or(state.swim_on),
        instance::ORBIT_SPEED => state.swim_value = n().unwrap_or(state.swim_value),
        instance::FLASHING_ON => state.flicker_on = b().unwrap_or(state.flicker_on),
        instance::FLASHING_SPEED => state.flicker_value = n().unwrap_or(state.flicker_value),
        _ => return false,
    }
    true
}

/// Apply a single auto-off control change onto the held auto-off state. Like
/// `apply_blob_field`, the three fields share one frame (33 30 05), so the
/// caller reads the held state, applies one field, and re-sends. Setting the
/// timeout also keeps `prev_minutes` in step so the from->to pair the app sends
/// stays consistent. Returns false for instances it doesn't own.
pub fn apply_auto_off_field(instance: &str, value: &JsonValue, state: &mut SetAutoOff) -> bool {
    let b = || value.as_bool().or_else(|| value.as_i64().map(|v| v != 0));
    let n = || value.as_i64().map(|v| v.clamp(0, 255) as u8);
    match instance {
        instance::AUTO_OFF_ENABLE => state.enable = b().unwrap_or(state.enable),
        instance::AUTO_OFF_STOP_SOUND => state.stop_sound = b().unwrap_or(state.stop_sound),
        instance::AUTO_OFF_MINUTES => {
            if let Some(minutes) = n() {
                state.prev_minutes = state.minutes;
                state.minutes = minutes;
            }
        }
        _ => return false,
    }
    true
}

/// The HA `entity_category` for a synthesized H6093 instance, if it is one of
/// ours. The aurora/stars on/off stay primary controls (None -> HA "Controls");
/// every other projector control (brightness/speed/effect/color-mode/settings/
/// auto-off) is `Some("config")` -> HA "Configuration", so the device page isn't
/// cluttered. Returns None for any instance we don't own, so callers leave other
/// devices' entities untouched.
pub fn entity_category(instance: &str) -> Option<Option<String>> {
    let primary = matches!(instance, instance::AURORA_ON | instance::STARS_ON);
    let ours = is_projector_instance(instance);
    if !ours {
        return None;
    }
    Some(if primary {
        None
    } else {
        Some("config".to_string())
    })
}

/// Whether `instance` is one of the H6093 projector controls we synthesize.
/// Mirrors the instances `apply_blob_field` / `apply_auto_off_field` /
/// `encode_capability` handle.
fn is_projector_instance(name: &str) -> bool {
    use instance::*;
    matches!(
        name,
        PAIRING_STATUS
            | PAIRING_SOUND
            | SILENT_POWER_UP
            | DREAMVIEW_LASER
            | AURORA_ON
            | AURORA_BRIGHTNESS
            | AURORA_EFFECT
            | AURORA_EFFECT_SPEED
            | AURORA_FLOW
            | AURORA_COLOR_MODE
            | STARS_ON
            | STARS_BRIGHTNESS
            | ORBIT_ON
            | ORBIT_SPEED
            | FLASHING_ON
            | FLASHING_SPEED
            | AUTO_OFF_ENABLE
            | AUTO_OFF_STOP_SOUND
            | AUTO_OFF_MINUTES
    )
}

/// Build the HA-facing state for one H6093 instance from the held aurora/laser
/// and auto-off state, as a `(kind, value)` pair the entity's `notify_state`
/// publishes (it reads `state["value"]`). Returns None for instances whose
/// current value we don't track (eg: the settings toggles, which the device
/// doesn't report back), so HA leaves those unknown rather than showing a guess.
/// This is the readback counterpart to `apply_blob_field`/`apply_auto_off_field`.
pub fn state_value(
    instance: &str,
    blob: &SetAuroraLaser,
    auto_off: &SetAutoOff,
) -> Option<(crate::model::DeviceCapabilityKind, JsonValue)> {
    use crate::model::DeviceCapabilityKind::{Mode, Range, Toggle};
    let on = |b: bool| (Toggle, serde_json::json!({ "value": i32::from(b) }));
    let num = |n: u8| (Range, serde_json::json!({ "value": n }));
    Some(match instance {
        instance::AURORA_ON => on(blob.aurora_on),
        instance::AURORA_COLOR_MODE => {
            let name = match blob.color_mode {
                AuroraColorMode::Basic => "Basic",
                AuroraColorMode::Advanced => "Advanced",
            };
            (Mode, serde_json::json!({ "value": name }))
        }
        instance::AURORA_BRIGHTNESS => num(blob.aurora_brightness),
        instance::AURORA_EFFECT_SPEED => num(blob.aurora_effect_speed),
        instance::AURORA_FLOW => num(blob.aurora_flow),
        instance::AURORA_EFFECT => (
            Mode,
            serde_json::json!({ "value": blob.aurora_effect_code }),
        ),
        instance::STARS_ON => on(blob.laser_on),
        instance::STARS_BRIGHTNESS => num(blob.laser_brightness),
        instance::ORBIT_ON => on(blob.swim_on),
        instance::ORBIT_SPEED => num(blob.swim_value),
        instance::FLASHING_ON => on(blob.flicker_on),
        instance::FLASHING_SPEED => num(blob.flicker_value),
        instance::AUTO_OFF_ENABLE => on(auto_off.enable),
        instance::AUTO_OFF_STOP_SOUND => on(auto_off.stop_sound),
        instance::AUTO_OFF_MINUTES => num(auto_off.minutes),
        _ => return None,
    })
}

/// An RGB color as the device packs it: three bytes, no flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub(super) fn register(codecs: &mut Vec<PacketCodec>) {
    // Settings-panel toggles: 33 30 <id> <on>. Each id is a distinct switch.
    codecs.push(packet!(
        &["H6093"],
        SetPairingStatus,
        SetPairingStatus,
        0x33,
        0x30,
        0x02,
        on,
    ));
    codecs.push(packet!(
        &["H6093"],
        SetPairingSound,
        SetPairingSound,
        0x33,
        0x30,
        0x03,
        on,
    ));
    codecs.push(packet!(
        &["H6093"],
        SetSilentPowerUp,
        SetSilentPowerUp,
        0x33,
        0x30,
        0x04,
        on,
    ));
    codecs.push(packet!(
        &["H6093"],
        SetDreamViewLaser,
        SetDreamViewLaser,
        0x33,
        0x30,
        0x07,
        on,
    ));

    // Auto-off: 33 30 05 <enable> <stop_sound> <minutes> <prev_minutes>.
    // minutes is the timeout 30-240 (raw); prev_minutes is the value it was
    // before this change (the app sends a from->to pair). For a write, set both
    // to the target.
    codecs.push(packet!(
        &["H6093"],
        SetAutoOff,
        SetAutoOff,
        0x33,
        0x30,
        0x05,
        enable,
        stop_sound,
        minutes,
        prev_minutes,
    ));

    // Aurora config: 33 11 <on> <speed> 0F 0F 01 03 1F <color_flag> <r> <g> <b> <enable>.
    // The 0F 0F 01 03 1F run is constant across every capture; color_flag and
    // enable were 01 whenever aurora was on. The 0xaa form is the status echo.
    codecs.push(packet!(
        &["H6093"],
        SetAurora,
        SetAurora,
        0x33,
        0x11,
        on,
        speed,
        0x0f,
        0x0f,
        0x01,
        0x03,
        0x1f,
        color_flag,
        r,
        g,
        b,
        enable,
    ));
    codecs.push(packet!(
        &["H6093"],
        NotifyAurora,
        NotifyAurora,
        0xaa,
        0x11,
        on,
        speed,
        0x0f,
        0x0f,
        0x01,
        0x03,
        0x1f,
        color_flag,
        r,
        g,
        b,
        enable,
    ));

    // Laser status read (aa 34). The laser is not written via a standalone
    // frame; it is part of the live aurora+laser blob below. This decodes the
    // device's compact status echo. byte2 carries the laser on/flags nibble.
    codecs.push(packet!(
        &["H6093"],
        NotifyLaser,
        NotifyLaser,
        0xaa,
        0x34,
        flags,
        swim,
        brightness,
        flicker,
    ));

    // The live aurora+laser realtime write is variable-length (count-prefixed
    // color arrays), so it is hand-encoded rather than declared via packet!.
    // It has no decode (the device reports state via the aa 11/12/23/34 frames).
    codecs.push(PacketCodec::new(
        &["H6093"],
        |v: &SetAuroraLaser| v.encode(),
        |_| anyhow::bail!("SetAuroraLaser has no decode; state comes from aa 11/12/23/34"),
    ));
}

/// 33 30 02 - pairing status (whether the device is discoverable for pairing)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SetPairingStatus {
    pub on: bool,
}

/// 33 30 03 - pairing sound (the chime played during pairing)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SetPairingSound {
    pub on: bool,
}

/// 33 30 04 - silent power up (suppress the startup sound)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SetSilentPowerUp {
    pub on: bool,
}

/// 33 30 07 - DreamView laser feature toggle
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SetDreamViewLaser {
    pub on: bool,
}

/// 33 30 05 - auto-off timer with a "stop playing sound" sub-option
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SetAutoOff {
    pub enable: bool,
    pub stop_sound: bool,
    pub minutes: u8,
    pub prev_minutes: u8,
}

impl SetAutoOff {
    /// Build an auto-off write. The app sends the new duration in `minutes` and
    /// the duration it was before in `prev_minutes`; the device only acts on
    /// `minutes`, so callers that don't track the previous value can pass the
    /// same value for both.
    pub fn new(enable: bool, stop_sound: bool, minutes: u8) -> Self {
        Self {
            enable,
            stop_sound,
            minutes,
            prev_minutes: minutes,
        }
    }
}

/// 33 11 write / aa 11 read - aurora (nebula) layer config
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SetAurora {
    pub on: bool,
    pub speed: u8,
    pub color_flag: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub enable: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
pub struct NotifyAurora {
    pub on: bool,
    pub speed: u8,
    pub color_flag: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub enable: u8,
}

/// aa 34 read - laser (stars) layer status. byte2 (`flags`) carries the laser
/// on state in its high bit, confirmed by correlating against the app's
/// laserIsOn writes. The remaining flag bits and the swim/brightness/flicker
/// value bytes use a packing distinct from the live aurora+laser write blob and
/// are not pinned to specific fields from the captures we have, so only the
/// on/off bit is interpreted here; the slider values are carried by our own
/// write state instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
pub struct NotifyLaser {
    pub flags: u8,
    pub swim: u8,
    pub brightness: u8,
    pub flicker: u8,
}

impl NotifyLaser {
    /// Whether the laser layer is on. Bit 7 of the flags byte; `flags == 0` is
    /// the all-off echo.
    pub fn is_on(&self) -> bool {
        self.flags & 0x80 != 0
    }
}

/// The live aurora+laser realtime write the app sends for slider/toggle control
/// (cmd:"ptReal"). It is a flat 15-byte head followed by two count-prefixed RGB
/// arrays (a coarse and a fine aurora segment), then chunked into 0xA3 frames
/// the same way `SetSceneCode` frames its scene data. The byte map was decoded by
/// correlating the wire bytes against the app's per-field state writes.
///
/// This is distinct from the 0x48 DIY-effect blob (Pro4H6093Diy), which is a
/// server-stored effect format, not the live control path.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SetAuroraLaser {
    /// byte[7]: the stars (laser) layer on/off. Confirmed via isolated capture
    /// (toggling stars in the app flips byte[7]). The head interleaves the layers:
    /// aurora on at [1], stars on at [7], with each layer's sliders around them.
    pub laser_on: bool,
    /// byte[2]: the laser (app: "stars") relative-brightness slider, 0-100.
    pub laser_brightness: u8,
    /// app label: "flashing"
    pub flicker_on: bool,
    pub flicker_value: u8,
    /// app label: "orbit"
    pub swim_on: bool,
    pub swim_value: u8,
    /// byte[1]: the aurora (nebula) layer on/off. Confirmed via isolated capture
    /// (toggling aurora in the app flips byte[1]).
    pub aurora_on: bool,
    pub aurora_flow: u8,
    /// byte[9]: the aurora (nebula) relative-brightness slider, 0-100. Distinct
    /// from the device master brightness, which is sent via cmd:"brightness".
    pub aurora_brightness: u8,
    /// 1-4
    pub aurora_effect_code: u8,
    pub aurora_effect_speed: u8,
    /// byte[12]: the aurora color mode. Decoded on the device: 0 = basic (a
    /// single `basic_colors` list), 1 = advanced (coarse "waves" + fine "flows"
    /// segments). The color tail layout after byte[12] depends on this.
    pub color_mode: AuroraColorMode,
    /// basic-mode color list (auroraColorArray); used when color_mode == Basic
    pub basic_colors: Vec<Rgb>,
    /// advanced-mode "waves" (coarse) segment on/off. In advanced mode each
    /// segment carries its own on-flag, count, and colors; a segment stays in the
    /// blob with its colors even when off. At least one of waves/flows must be on.
    pub coarse_on: bool,
    /// advanced-mode coarse segment ("waves") colors; used when Advanced
    pub coarse_colors: Vec<Rgb>,
    /// advanced-mode "flows" (fine) segment on/off. See coarse_on.
    pub fine_on: bool,
    /// advanced-mode fine segment ("flows") colors; used when Advanced
    pub fine_colors: Vec<Rgb>,
}

/// The aurora color mode (byte[12] of the live blob). Determines the color tail
/// layout: Basic carries one color list, Advanced carries coarse + fine segments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AuroraColorMode {
    #[default]
    Basic,
    Advanced,
}

impl SetAuroraLaser {
    /// controlType 0x0C: the segmented (FenKong) aurora layout this write uses.
    const CONTROL_TYPE: u8 = 0x0C;

    fn encode(&self) -> anyhow::Result<Vec<u8>> {
        // Head byte map (research/api-map/07-frame-reference.md). Aurora on/off is
        // byte[1], stars on/off is byte[7].
        let mut payload = vec![
            Self::CONTROL_TYPE,        // [0]
            u8::from(self.aurora_on),  // [1] aurora layer on/off
            self.laser_brightness,     // [2] stars relative brightness
            u8::from(self.flicker_on), // [3] flashing on/off
            self.flicker_value,        // [4] flashing speed
            u8::from(self.swim_on),    // [5] orbit on/off
            self.swim_value,           // [6] orbit speed
            u8::from(self.laser_on),   // [7] stars layer on/off
            self.aurora_flow,          // [8] aurora flow rate
            self.aurora_brightness,    // [9] aurora relative brightness
            self.aurora_effect_code, // [10] aurora mode (1=Gradient 2=Breathe 4=Rainbow 3=Twinkle)
            self.aurora_effect_speed, // [11] aurora speed
            match self.color_mode {
                AuroraColorMode::Basic => 0,
                AuroraColorMode::Advanced => 1,
            },
        ];
        // Color tail layout by mode:
        //   Basic:    [count][count x RGB]
        //   Advanced: [wavesOn][wavesN][waves RGB] [flowsOn][flowsN][flows RGB]
        // Advanced "waves"=coarse, "flows"=fine; each segment carries its on-flag,
        // count, and colors, and keeps its colors when its on-flag is 0.
        match self.color_mode {
            AuroraColorMode::Basic => {
                payload.push(self.basic_colors.len() as u8);
                for c in &self.basic_colors {
                    payload.extend_from_slice(&[c.r, c.g, c.b]);
                }
            }
            AuroraColorMode::Advanced => {
                for (on, colors) in [
                    (self.coarse_on, &self.coarse_colors),
                    (self.fine_on, &self.fine_colors),
                ] {
                    payload.push(u8::from(on));
                    payload.push(colors.len() as u8);
                    for c in colors {
                        payload.extend_from_slice(&[c.r, c.g, c.b]);
                    }
                }
            }
        }
        let mut frames = frame_a3(&payload);
        // The app terminates the A3 blob with a `33 05 <controlType>` frame that
        // tells the device the multi-frame write is complete and to apply it.
        // Without it the device receives the data but never commits the change.
        frames.extend(finish(vec![0x33, 0x05, Self::CONTROL_TYPE]));
        Ok(frames)
    }

    /// Build the held aurora+laser state from the app's stored common-datas JSON
    /// (the `bizType:3` blob, read via `GoveeUndocumentedApi::get_common_datas`).
    /// This is the seed source for single-field edits: the named JSON fields are
    /// the app's own representation, so no byte-mapping is involved. Missing
    /// fields keep their default. The advanced-mode "waves"/"flows" segments map
    /// to coarse/fine; basic-mode `auroraColorArray` is the unified color list.
    pub fn from_common_datas(v: &JsonValue) -> Self {
        let u8f = |key: &str| v.get(key).and_then(|x| x.as_i64()).map(|n| n as u8);
        let boolf = |key: &str| v.get(key).and_then(|x| x.as_bool());
        let colors = |key: &str| -> Vec<Rgb> {
            v.get(key)
                .and_then(|x| x.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| {
                            let rgb = c.as_array()?;
                            Some(Rgb {
                                r: rgb.first()?.as_i64()? as u8,
                                g: rgb.get(1)?.as_i64()? as u8,
                                b: rgb.get(2)?.as_i64()? as u8,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let d = Self::default();
        Self {
            aurora_on: boolf("auroraIsOn").unwrap_or(d.aurora_on),
            laser_brightness: u8f("laserBrightnessValue").unwrap_or(d.laser_brightness),
            flicker_on: boolf("laserflickerIsOn").unwrap_or(d.flicker_on),
            flicker_value: u8f("laserflickerValue").unwrap_or(d.flicker_value),
            swim_on: boolf("laserSwimIsOn").unwrap_or(d.swim_on),
            swim_value: u8f("laserSwimValue").unwrap_or(d.swim_value),
            laser_on: boolf("laserIsOn").unwrap_or(d.laser_on),
            aurora_flow: u8f("auroraFlowValue").unwrap_or(d.aurora_flow),
            aurora_brightness: u8f("auroraBrightnessValue").unwrap_or(d.aurora_brightness),
            aurora_effect_code: u8f("auroraEffectCode").unwrap_or(d.aurora_effect_code),
            aurora_effect_speed: u8f("auroraEffectSpeedValue").unwrap_or(d.aurora_effect_speed),
            // auroraIsHigh selects the color mode: false = basic, true = advanced.
            color_mode: match boolf("auroraIsHigh") {
                Some(true) => AuroraColorMode::Advanced,
                _ => AuroraColorMode::Basic,
            },
            // basic mode uses auroraColorArray; advanced uses coarse "waves" +
            // fine "flows", each with its own on-flag. We keep all three color
            // lists plus the two flags so the held state round-trips the device's
            // full color picture regardless of which mode is active. A missing
            // on-flag defaults on: a segment is normally on, and both-off is an
            // invalid advanced state the device rejects.
            coarse_on: boolf("auroraCoarseIsOn").unwrap_or(true),
            fine_on: boolf("auroraFineIsOn").unwrap_or(true),
            basic_colors: colors("auroraColorArray"),
            coarse_colors: colors("auroraCoarseColorArray"),
            fine_colors: colors("auroraFineColorArray"),
        }
    }
}

/// Chunk a payload into 0xA3 frames the way the device expects: a header frame
/// `A3 00 01 <nlines>` carrying the first 15 payload bytes, then `A3 <line>`
/// frames carrying 17 bytes each, the last line marked 0xFF. Each frame is
/// padded to 19 bytes with an XOR checksum as byte 20 (via `finish`). Mirrors
/// the framing in `light::SetSceneCode::encode`.
fn frame_a3(payload: &[u8]) -> Vec<u8> {
    let mut data = vec![0xa3, 0x00, 0x01, 0x00 /* line count, back-patched */];
    let mut num_lines = 0u8;
    let mut last_line_marker = 1;

    for &b in payload {
        if data.len().is_multiple_of(19) {
            num_lines += 1;
            data.push(0xa3);
            last_line_marker = data.len();
            data.push(num_lines);
        }
        data.push(b);
    }
    data[last_line_marker] = 0xff;
    data[3] = num_lines + 1;

    let mut out = vec![];
    for chunk in data.chunks(19) {
        out.append(&mut finish(chunk.to_vec()));
    }
    out
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ble::Base64HexBytes;
    use crate::ble::codec::GoveeBlePacket;

    /// Encode a typed command for the H6093 and return the raw frame bytes.
    /// The expected values are the exact frames captured from the app's MQTT
    /// control stream; encoding must reproduce them
    /// byte-for-byte, including the trailing XOR checksum.
    fn enc<T: 'static>(value: &T) -> Vec<u8> {
        Base64HexBytes::encode_for_sku("H6093", value).unwrap().0.0
    }

    #[test]
    fn settings_toggles_match_capture() {
        // 33 30 02 01 ... checksum 00
        assert_eq!(
            enc(&SetPairingStatus { on: true }),
            vec![
                0x33, 0x30, 0x02, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x00
            ]
        );
        // 33 30 03 01 ... checksum 01
        assert_eq!(
            enc(&SetPairingSound { on: true }),
            vec![
                0x33, 0x30, 0x03, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01
            ]
        );
        // 33 30 04 01 ... checksum 06
        assert_eq!(
            enc(&SetSilentPowerUp { on: true }),
            vec![
                0x33, 0x30, 0x04, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x06
            ]
        );
        // 33 30 07 01 ... checksum 05
        assert_eq!(
            enc(&SetDreamViewLaser { on: true }),
            vec![
                0x33, 0x30, 0x07, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x05
            ]
        );
    }

    #[test]
    fn auto_off_matches_capture() {
        // captured: 33 30 05 01 01 F0 F0 ... checksum 06 (enable, stop_sound, 240, prev 240)
        assert_eq!(
            enc(&SetAutoOff::new(true, true, 240)),
            vec![
                0x33, 0x30, 0x05, 1, 1, 0xF0, 0xF0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x06
            ]
        );
        // captured: 33 30 05 01 01 1E F0 ... (30 min, prev 240); use the from->to pair
        assert_eq!(
            enc(&SetAutoOff {
                enable: true,
                stop_sound: true,
                minutes: 30,
                prev_minutes: 240,
            }),
            vec![
                0x33, 0x30, 0x05, 1, 1, 0x1E, 0xF0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xE8
            ]
        );
    }

    #[test]
    fn aurora_matches_capture() {
        // captured: 33 11 01 0A 0F 0F 01 03 1F 01 FF 07 CE 01 ... checksum 02
        assert_eq!(
            enc(&SetAurora {
                on: true,
                speed: 0x0A,
                color_flag: 0x01,
                r: 0xFF,
                g: 0x07,
                b: 0xCE,
                enable: 0x01,
            }),
            vec![
                0x33, 0x11, 0x01, 0x0A, 0x0F, 0x0F, 0x01, 0x03, 0x1F, 0x01, 0xFF, 0x07, 0xCE, 0x01,
                0, 0, 0, 0, 0, 0x02
            ]
        );
    }

    #[test]
    fn aurora_status_round_trips() {
        // the aa 11 status echo decodes back into NotifyAurora
        let bytes = Base64HexBytes::with_bytes(vec![
            0xAA, 0x11, 0x01, 0x0A, 0x0F, 0x0F, 0x01, 0x03, 0x1F, 0x01, 0xFF, 0x07, 0xCE, 0x01,
        ]);
        assert_eq!(
            bytes.decode_for_sku("H6093"),
            GoveeBlePacket::NotifyAurora(NotifyAurora {
                on: true,
                speed: 0x0A,
                color_flag: 0x01,
                r: 0xFF,
                g: 0x07,
                b: 0xCE,
                enable: 0x01,
            })
        );
    }

    #[test]
    fn aurora_laser_live_blob_matches_capture() {
        // Captured advanced-mode blob. Reassembled payload (A3 markers + checksums
        // stripped):
        //   0C 01 64 01 59 01 55 01 13 64 01 45 01   head ([12]=01 advanced)
        //   01 03 FF00B5 FF07E3 00F0FF                waves on, 3 colors
        //   01 03 FF0000 00FF00 FF07FF                flows on, 3 colors
        let cmd = SetAuroraLaser {
            laser_on: true,
            laser_brightness: 0x64,
            flicker_on: true,
            flicker_value: 0x59,
            swim_on: true,
            swim_value: 0x55,
            aurora_on: true,
            aurora_flow: 0x13,
            aurora_brightness: 0x64,
            aurora_effect_code: 0x01,
            aurora_effect_speed: 0x45,
            color_mode: AuroraColorMode::Advanced,
            basic_colors: vec![],
            coarse_on: true,
            coarse_colors: vec![
                Rgb {
                    r: 0xFF,
                    g: 0x00,
                    b: 0xB5,
                },
                Rgb {
                    r: 0xFF,
                    g: 0x07,
                    b: 0xE3,
                },
                Rgb {
                    r: 0x00,
                    g: 0xF0,
                    b: 0xFF,
                },
            ],
            fine_on: true,
            fine_colors: vec![
                Rgb {
                    r: 0xFF,
                    g: 0x00,
                    b: 0x00,
                },
                Rgb {
                    r: 0x00,
                    g: 0xFF,
                    b: 0x00,
                },
                Rgb {
                    r: 0xFF,
                    g: 0x07,
                    b: 0xFF,
                },
            ],
        };
        let got = enc(&cmd);
        // Assert on the reassembled PAYLOAD (the bytes the device parses), not the
        // exact frame boundaries: the device's frame-splitting rule isn't pinned
        // from a single sample (see the framing note on frame_a3), so we don't
        // assert on it here.
        #[rustfmt::skip]
        let expect_payload: Vec<u8> = vec![
            0x0C, 0x01, 0x64, 0x01, 0x59, 0x01, 0x55, 0x01, 0x13, 0x64, 0x01, 0x45, 0x01,
            0x01, 0x03, 0xFF, 0x00, 0xB5, 0xFF, 0x07, 0xE3, 0x00, 0xF0, 0xFF, // waves on, 3
            0x01, 0x03, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0xFF, 0x07, 0xFF, // flows on, 3
        ];
        assert_eq!(reassemble_a3(&got), expect_payload);

        // The blob must end with the `33 05 0C` apply-terminator frame; without
        // it the device receives the data but never commits the change.
        let trailer = &got[got.len() - 20..];
        assert_eq!(trailer[..3], [0x33, 0x05, 0x0C]);
    }

    #[test]
    fn aurora_and_stars_on_off_are_distinct_bytes() {
        // Aurora on is byte[1], stars on is byte[7]. Checked independently
        // (aurora on / stars off, then the inverse) so each byte is pinned; a
        // both-on fixture wouldn't tell the two apart.
        let cmd = SetAuroraLaser {
            aurora_on: true,
            laser_on: false,
            color_mode: AuroraColorMode::Basic,
            ..Default::default()
        };
        let payload = reassemble_a3(&enc(&cmd));
        assert_eq!(payload[1], 1, "byte[1] is aurora on/off");
        assert_eq!(payload[7], 0, "byte[7] is stars on/off");

        // and the inverse
        let cmd = SetAuroraLaser {
            aurora_on: false,
            laser_on: true,
            color_mode: AuroraColorMode::Basic,
            ..Default::default()
        };
        let payload = reassemble_a3(&enc(&cmd));
        assert_eq!(payload[1], 0);
        assert_eq!(payload[7], 1);
    }

    #[test]
    fn aurora_flow_and_change_speed_are_distinct_bytes() {
        // The two aurora sliders are distinct bytes: flow rate is byte[8], change
        // speed (effect speed) is byte[11].
        let cmd = SetAuroraLaser {
            aurora_on: true,
            aurora_flow: 52,
            aurora_effect_speed: 37,
            color_mode: AuroraColorMode::Basic,
            ..Default::default()
        };
        let payload = reassemble_a3(&enc(&cmd));
        assert_eq!(payload[8], 52, "byte[8] is aurora flow rate");
        assert_eq!(payload[11], 37, "byte[11] is aurora change speed");
    }

    #[test]
    fn basic_color_mode_blob_matches_capture() {
        // Captured basic-mode blob (20:23:15): byte[12]=00 (basic), then a count
        // and the auroraColorArray colors (red, blue). The advanced coarse/fine
        // tail must NOT appear.
        //   0C 01 32 00 46 01 12 01 13 50 01 45 00 02 FF 00 00 00 00 FF
        let cmd = SetAuroraLaser {
            laser_on: true,
            laser_brightness: 0x32,
            flicker_on: false,
            flicker_value: 0x46,
            swim_on: true,
            swim_value: 0x12,
            aurora_on: true,
            aurora_flow: 0x13,
            aurora_brightness: 0x50,
            aurora_effect_code: 0x01,
            aurora_effect_speed: 0x45,
            color_mode: AuroraColorMode::Basic,
            basic_colors: vec![
                Rgb {
                    r: 0xFF,
                    g: 0,
                    b: 0,
                },
                Rgb {
                    r: 0,
                    g: 0,
                    b: 0xFF,
                },
            ],
            coarse_on: true,
            coarse_colors: vec![Rgb { r: 1, g: 2, b: 3 }], // present but unused in basic
            fine_on: true,
            fine_colors: vec![],
        };
        #[rustfmt::skip]
        let expect: Vec<u8> = vec![
            0x0C, 0x01, 0x32, 0x00, 0x46, 0x01, 0x12, 0x01, 0x13, 0x50, 0x01, 0x45,
            0x00,             // byte[12] = basic
            0x02,             // count
            0xFF, 0x00, 0x00, // red
            0x00, 0x00, 0xFF, // blue
        ];
        assert_eq!(reassemble_a3(&enc(&cmd)), expect);
    }

    /// Inverse of `frame_a3`: pull the payload back out of the framed bytes,
    /// dropping the A3 line markers and checksums, and trimming the trailing zero
    /// frame-padding to the true payload length. The length depends on byte[12]
    /// (color mode): basic = 13 head + 1 count + 3*count; advanced = 13 head +
    /// 2 (wavesOn,wavesN) + 3*wavesN + 2 (flowsOn,flowsN) + 3*flowsN.
    fn reassemble_a3(framed: &[u8]) -> Vec<u8> {
        let mut logical = vec![];
        let mut first = true;
        for frame in framed.chunks(20) {
            // Only the A3-led frames carry payload; the trailing 33 05 frame is
            // the "apply" terminator and isn't part of the payload.
            if frame[0] != 0xa3 {
                continue;
            }
            let body = &frame[..frame.len().saturating_sub(1)]; // drop checksum
            if first {
                logical.extend_from_slice(&body[4..]); // skip A3 00 01 nlines
                first = false;
            } else {
                logical.extend_from_slice(&body[2..]); // skip A3 marker
            }
        }
        let len = if logical[12] == 0 {
            // basic: head [0..12] (13), byte[13] count, then count RGBs
            14 + 3 * logical[13] as usize
        } else {
            // advanced: head (13), [13]=wavesOn [14]=wavesN + waves RGBs, then
            // [.]=flowsOn [.+1]=flowsN + flows RGBs right after the waves colors.
            let waves_n = logical[14] as usize;
            let flows_n = logical[16 + 3 * waves_n] as usize;
            17 + 3 * (waves_n + flows_n)
        };
        logical.truncate(len);
        logical
    }

    #[test]
    fn laser_status_decodes() {
        // aa 34 status: flags swim brightness flicker
        let bytes = Base64HexBytes::with_bytes(vec![0xAA, 0x34, 0xC4, 0x54, 0xA0, 0x03]);
        assert_eq!(
            bytes.decode_for_sku("H6093"),
            GoveeBlePacket::NotifyLaser(NotifyLaser {
                flags: 0xC4,
                swim: 0x54,
                brightness: 0xA0,
                flicker: 0x03,
            })
        );
    }

    #[test]
    fn encode_capability_routes_instance_to_frame() {
        use serde_json::json;
        // The synthesized capability's instance name routes through the registry
        // to the right frame; an ON pairingStatus must produce 33 30 02 01.
        let frames = encode_capability("H6093", instance::PAIRING_STATUS, &json!(true))
            .expect("instance is handled")
            .expect("encodes");
        let bytes = data_encoding::BASE64.decode(frames[0].as_bytes()).unwrap();
        assert_eq!(bytes[..4], [0x33, 0x30, 0x02, 0x01]);

        // An unknown instance falls through (None) so the platform path runs.
        assert!(encode_capability("H6093", "noSuchInstance", &json!(true)).is_none());
        // A non-H6093 SKU is never frame-encoded here.
        assert!(encode_capability("H5082", instance::PAIRING_STATUS, &json!(true)).is_none());
    }

    #[test]
    fn apply_blob_field_changes_one_field_preserves_rest() {
        use serde_json::json;
        let mut state = SetAuroraLaser {
            aurora_on: true,
            laser_brightness: 50,
            flicker_value: 89,
            swim_value: 85,
            aurora_brightness: 60,
            aurora_effect_code: 2,
            coarse_colors: vec![Rgb { r: 1, g: 2, b: 3 }],
            ..Default::default()
        };

        // Changing orbit speed updates only swim_value; everything else holds,
        // which is what lets a single-slider change re-send a correct full frame.
        let before = state.clone();
        assert!(apply_blob_field(
            instance::ORBIT_SPEED,
            &json!(30),
            &mut state
        ));
        assert_eq!(state.swim_value, 30);
        assert_eq!(
            SetAuroraLaser {
                swim_value: before.swim_value,
                ..state.clone()
            },
            before
        );

        // Stars vs aurora relative brightness map to distinct fields.
        apply_blob_field(instance::STARS_BRIGHTNESS, &json!(10), &mut state);
        apply_blob_field(instance::AURORA_BRIGHTNESS, &json!(90), &mut state);
        assert_eq!(state.laser_brightness, 10);
        assert_eq!(state.aurora_brightness, 90);

        // An instance we don't own returns false (caller falls through).
        assert!(!apply_blob_field("powerSwitch", &json!(true), &mut state));
    }

    #[test]
    fn apply_auto_off_field_tracks_prev_minutes() {
        use serde_json::json;
        let mut state = SetAutoOff::new(true, false, 240);

        // Changing the timeout records the old value in prev_minutes (the
        // from->to pair the app sends) and updates minutes.
        assert!(apply_auto_off_field(
            instance::AUTO_OFF_MINUTES,
            &json!(30),
            &mut state
        ));
        assert_eq!(state.minutes, 30);
        assert_eq!(state.prev_minutes, 240);

        // The two sub-toggles flip only their own field.
        apply_auto_off_field(instance::AUTO_OFF_STOP_SOUND, &json!(true), &mut state);
        assert!(state.stop_sound);
        assert_eq!(state.minutes, 30); // unchanged
        apply_auto_off_field(instance::AUTO_OFF_ENABLE, &json!(false), &mut state);
        assert!(!state.enable);

        // The frame it produces matches the capture layout: 33 30 05 en snd min prev.
        let bytes = Base64HexBytes::encode_for_sku("H6093", &state).unwrap().0.0;
        assert_eq!(bytes[..7], [0x33, 0x30, 0x05, 0x00, 0x01, 30, 240]);

        assert!(!apply_auto_off_field(
            "powerSwitch",
            &json!(true),
            &mut state
        ));
    }

    #[test]
    fn seeds_from_common_datas() {
        use serde_json::json;
        // The shape the app stores (and we read via get_common_datas): named
        // fields, with advanced-mode "waves"=coarse and "flows"=fine.
        let blob = json!({
            "auroraIsOn": true,
            "auroraBrightnessValue": 80,
            "auroraEffectCode": 3,
            "auroraEffectSpeedValue": 69,
            "auroraFlowValue": 19,
            "auroraIsHigh": true,
            "laserIsOn": true,
            "laserBrightnessValue": 50,
            "laserSwimIsOn": true,
            "laserSwimValue": 80,
            "laserflickerIsOn": false,
            "laserflickerValue": 70,
            "auroraCoarseColorArray": [[0, 255, 0], [0, 225, 255]],
            "auroraFineColorArray": [[255, 0, 0]],
        });
        let s = SetAuroraLaser::from_common_datas(&blob);
        assert!(s.aurora_on);
        assert_eq!(s.aurora_brightness, 80);
        assert_eq!(s.aurora_effect_code, 3);
        assert_eq!(s.laser_brightness, 50);
        assert_eq!(s.swim_value, 80);
        assert!(!s.flicker_on);
        assert_eq!(s.flicker_value, 70);
        assert_eq!(
            s.coarse_colors,
            vec![
                Rgb { r: 0, g: 255, b: 0 },
                Rgb {
                    r: 0,
                    g: 225,
                    b: 255
                }
            ]
        );
        assert_eq!(s.fine_colors, vec![Rgb { r: 255, g: 0, b: 0 }]);

        // Missing fields keep the default rather than erroring.
        let partial = SetAuroraLaser::from_common_datas(&json!({"auroraIsOn": true}));
        assert!(partial.aurora_on);
        assert_eq!(partial.aurora_brightness, 0);
    }

    #[test]
    fn entity_category_controls_vs_config() {
        // The two layer on/offs are primary controls (no category -> HA Controls).
        assert_eq!(entity_category(instance::AURORA_ON), Some(None));
        assert_eq!(entity_category(instance::STARS_ON), Some(None));
        // Everything else projector goes to Configuration.
        for inst in [
            instance::AURORA_BRIGHTNESS,
            instance::AURORA_EFFECT,
            instance::AURORA_EFFECT_SPEED,
            instance::AURORA_FLOW,
            instance::AURORA_COLOR_MODE,
            instance::STARS_BRIGHTNESS,
            instance::ORBIT_ON,
            instance::ORBIT_SPEED,
            instance::FLASHING_ON,
            instance::FLASHING_SPEED,
            instance::PAIRING_STATUS,
            instance::PAIRING_SOUND,
            instance::SILENT_POWER_UP,
            instance::DREAMVIEW_LASER,
            instance::AUTO_OFF_ENABLE,
            instance::AUTO_OFF_STOP_SOUND,
            instance::AUTO_OFF_MINUTES,
        ] {
            assert_eq!(
                entity_category(inst),
                Some(Some("config".to_string())),
                "{inst} should be config"
            );
        }
        // Non-projector instances are left untouched (None -> caller keeps its default).
        assert_eq!(entity_category("powerSwitch"), None);
        assert_eq!(entity_category("brightness"), None);
    }
}
