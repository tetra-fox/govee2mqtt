//! BLE/IoT command structs for the H5080-family smart plugs. Today this covers
//! the H5082 dual-outlet plug: countdowns (the device's "auto-on"/"auto-off"),
//! recurring timers, and the per-outlet timer-count read. Layouts decoded from
//! `research/mitm/H5082-protocol.md`, validated against the btsnoop captures in
//! `research/mitm/h5082-*.btsnoop` and the IoT-side `op.command` entries in
//! `research/iot-trace.log`.
//!
//! The H5082's IoT relay carries the same 0x33/0xaa BLE frames base64-wrapped
//! inside the cloud `cmd:"ptReal"` payload (see memory/iot-wraps-ble.md), so
//! the codecs here serve both transports.
//!
//! Outlet indexing: the wire byte is 0x01 for outlet 1, 0x00 for outlet 2.
//! This is the inverse of the `socket_turn_val` selector and matches the
//! `aa b0` connection-time poll order. Higher layers translate to/from the
//! user-facing 1-based outlet number.

use super::codec::{Base64HexBytes, DecodePacketParam, PacketCodec, finish};
use super::family::{FamilyModule, SyncClock};
use crate::error::ApiResult;
use crate::packet;
use serde_json::Value as JsonValue;

const SUPPORTED_SKUS: &[&str] = &["H5082"];

/// User-facing instance names. Outlet numbers in the names are 1-based to match
/// the app and the existing per-outlet switches; the wire byte is the inverse
/// (outlet 1 = 0x01, outlet 2 = 0x00) and is translated at the codec boundary.
///
/// Each `(outlet, kind)` slot has two paired instances: a read-only
/// `*Remaining` sensor (seconds) and a writable `*Duration` Number (minutes).
/// Writing a non-zero value to the Duration arms the countdown; writing 0
/// disarms it.
pub mod instance {
    pub const O1_AUTO_ON_REMAINING: &str = "outlet1AutoOnRemaining";
    pub const O1_AUTO_OFF_REMAINING: &str = "outlet1AutoOffRemaining";
    pub const O2_AUTO_ON_REMAINING: &str = "outlet2AutoOnRemaining";
    pub const O2_AUTO_OFF_REMAINING: &str = "outlet2AutoOffRemaining";

    pub const O1_AUTO_ON_DURATION: &str = "outlet1AutoOnDuration";
    pub const O1_AUTO_OFF_DURATION: &str = "outlet1AutoOffDuration";
    pub const O2_AUTO_ON_DURATION: &str = "outlet2AutoOnDuration";
    pub const O2_AUTO_OFF_DURATION: &str = "outlet2AutoOffDuration";
}

/// Translate the user-facing 1-based outlet number to the wire byte the device
/// uses for `aa b0`/`33 b0` (`1 -> 0x01`, `2 -> 0x00`). Returns `None` for any
/// other input so callers can fall through cleanly.
pub fn outlet_wire(user_outlet: u8) -> Option<u8> {
    match user_outlet {
        1 => Some(0x01),
        2 => Some(0x00),
        _ => None,
    }
}

/// Read-side of `instance_to_slot`: which `(outlet, kind)` does this
/// `*Remaining` sensor describe? `kind_wire` is the wire value
/// (`0x00` = fire-OFF, `0x01` = fire-ON).
fn remaining_instance_to_slot(instance: &str) -> Option<(u8, u8)> {
    Some(match instance {
        instance::O1_AUTO_ON_REMAINING => (0x01, 0x01),
        instance::O1_AUTO_OFF_REMAINING => (0x01, 0x00),
        instance::O2_AUTO_ON_REMAINING => (0x00, 0x01),
        instance::O2_AUTO_OFF_REMAINING => (0x00, 0x00),
        _ => return None,
    })
}

/// Write-side of `instance_to_slot`: which slot does this `*Duration` Number
/// arm or disarm?
fn duration_instance_to_slot(instance: &str) -> Option<(u8, u8)> {
    Some(match instance {
        instance::O1_AUTO_ON_DURATION => (0x01, 0x01),
        instance::O1_AUTO_OFF_DURATION => (0x01, 0x00),
        instance::O2_AUTO_ON_DURATION => (0x00, 0x01),
        instance::O2_AUTO_OFF_DURATION => (0x00, 0x00),
        _ => return None,
    })
}

/// Any instance this family owns, for the entity_category and entity_name
/// dispatch.
fn owned_instance(instance: &str) -> bool {
    remaining_instance_to_slot(instance).is_some() || duration_instance_to_slot(instance).is_some()
}

/// Parse a Duration Number's HA-side value into a clamped 0..1439 minute
/// total. Accepts both `Value::Number` (the i64/f64 path) and
/// `Value::String` (some MQTT discovery clients quote payloads), so a
/// stringified "30" round-trips the same as a numeric `30`. Anything
/// unparsable becomes 0 (disarm).
fn parse_duration_minutes(value: &JsonValue) -> u32 {
    let n = value
        .as_i64()
        .or_else(|| value.as_f64().map(|f| f.round() as i64))
        .or_else(|| value.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
        .unwrap_or(0);
    n.clamp(0, 23 * 60 + 59) as u32
}

/// Parse a timer-slot write request from the MQTT JSON payload into the
/// typed `SetTimerSlot` frame. The shape:
///
/// ```json
/// {
///   "outlet":  1 | 2,
///   "slot":    0..N,
///   "kind":    "on" | "off",
///   "time":    "HH:MM",
///   "days":    ["mon","tue",...] | "all" | [],
///   "enabled": true | false   // optional, default true
/// }
/// ```
///
/// Returns a structured parse error so the handler can reply meaningfully
/// rather than a generic 400.
pub fn parse_timer_request(payload: &JsonValue) -> Result<SetTimerSlot, TimerParseError> {
    let outlet_n = payload
        .get("outlet")
        .and_then(|v| v.as_i64())
        .ok_or(TimerParseError::MissingOutlet)?;
    let outlet = outlet_wire(outlet_n.try_into().unwrap_or(0)).ok_or(TimerParseError::BadOutlet)?;

    let slot = payload
        .get("slot")
        .and_then(|v| v.as_i64())
        .ok_or(TimerParseError::MissingSlot)?;
    if !(0..=255).contains(&slot) {
        return Err(TimerParseError::BadSlot);
    }
    let slot = slot as u8;

    let kind_str = payload
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or(TimerParseError::MissingKind)?;
    let kind_bit = match kind_str {
        "on" | "ON" | "On" => 1u8,
        "off" | "OFF" | "Off" => 0u8,
        _ => return Err(TimerParseError::BadKind),
    };

    let time_str = payload
        .get("time")
        .and_then(|v| v.as_str())
        .ok_or(TimerParseError::MissingTime)?;
    let (hh, mm) = parse_hhmm(time_str).ok_or(TimerParseError::BadTime)?;

    // Days: array of weekday names, or "all" for the every-day sentinel.
    // Empty array also means "every-day sentinel" by convention.
    let days = match payload.get("days") {
        Some(JsonValue::String(s)) if s.eq_ignore_ascii_case("all") => 0x00u8,
        Some(JsonValue::Array(arr)) if arr.is_empty() => 0x00u8,
        Some(JsonValue::Array(arr)) => {
            let mut mask: u8 = 0;
            for d in arr {
                let name = d.as_str().ok_or(TimerParseError::BadDays)?;
                mask |= day_bit(name).ok_or(TimerParseError::BadDays)?;
            }
            // bit 7 = "selective" flag; required whenever a non-zero day mask
            // is sent so the device picks the day-mask interpretation rather
            // than the "every day" sentinel.
            mask | 0x80
        }
        None => return Err(TimerParseError::MissingDays),
        _ => return Err(TimerParseError::BadDays),
    };

    let enabled = payload
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    // flags byte: bit 7 = enable, bit 0 = type (1 = fire-on, 0 = fire-off).
    let flags = (if enabled { 0x80 } else { 0x00 }) | kind_bit;

    Ok(SetTimerSlot {
        outlet,
        slot,
        flags,
        hh,
        mm,
        days,
    })
}

/// Parse `"HH:MM"` (e.g. `"17:11"`, `"7:5"`) into a `(hh, mm)` pair with
/// both fields in their device-side ranges. Returns `None` for malformed
/// input or out-of-range values.
fn parse_hhmm(s: &str) -> Option<(u8, u8)> {
    let (h, m) = s.split_once(':')?;
    let hh: u8 = h.trim().parse().ok()?;
    let mm: u8 = m.trim().parse().ok()?;
    if hh >= 24 || mm >= 60 {
        return None;
    }
    Some((hh, mm))
}

/// Map a weekday name to its bit position in the H5082's day mask.
/// Mon=bit0..Sun=bit6.
fn day_bit(name: &str) -> Option<u8> {
    Some(match name.to_ascii_lowercase().as_str() {
        "mon" | "monday" => 1 << 0,
        "tue" | "tues" | "tuesday" => 1 << 1,
        "wed" | "weds" | "wednesday" => 1 << 2,
        "thu" | "thur" | "thurs" | "thursday" => 1 << 3,
        "fri" | "friday" => 1 << 4,
        "sat" | "saturday" => 1 << 5,
        "sun" | "sunday" => 1 << 6,
        _ => return None,
    })
}

/// Parse error variants for [`parse_timer_request`]. Each variant maps to a
/// distinct user-facing message so a misshaped payload tells the caller
/// exactly which field to fix.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TimerParseError {
    #[error("missing 'outlet'")]
    MissingOutlet,
    #[error("'outlet' must be 1 or 2")]
    BadOutlet,
    #[error("missing 'slot'")]
    MissingSlot,
    #[error("'slot' must be 0..255")]
    BadSlot,
    #[error("missing 'kind'")]
    MissingKind,
    #[error("'kind' must be \"on\" or \"off\"")]
    BadKind,
    #[error("missing 'time'")]
    MissingTime,
    #[error("'time' must be \"HH:MM\" with HH in 0..23 and MM in 0..59")]
    BadTime,
    #[error("missing 'days'")]
    MissingDays,
    #[error("'days' must be an array of weekday names, [] or \"all\" for every day")]
    BadDays,
}

/// Optimistic post-write hook: given the instance and value the user just
/// wrote, return the `NotifyCountdown` that should be inserted into the
/// device's held state so HA's state-topic readback reflects the new value
/// before the device's next status broadcast arrives. `None` for any
/// instance this family does not own.
///
/// Note: `seconds_remaining` is left at 0 because we don't know the
/// device's wall-clock at the moment we record; the real value arrives on
/// the next status broadcast. HA shows the preset duration immediately,
/// and the live seconds populate when the broadcast lands.
pub fn record_optimistic_write(instance: &str, value: &JsonValue) -> Option<NotifyCountdown> {
    let (outlet, kind) = duration_instance_to_slot(instance)?;
    let minutes_total = parse_duration_minutes(value);
    Some(NotifyCountdown {
        outlet,
        kind,
        hh: (minutes_total / 60) as u8,
        mm: (minutes_total % 60) as u8,
        seconds_remaining: SecondsRemainingBe24(0),
    })
}

/// HA-facing state for a synthesized H5082 instance, given the device's held
/// `(outlet, kind) -> NotifyCountdown` map. Returns `(kind, json!({"value": N}))`
/// for any instance this family owns; `None` for instances it does not. The
/// caller (in `service/device.rs`) chains this after the projector family.
pub fn state_value(
    instance: &str,
    countdowns: &std::collections::HashMap<(u8, u8), NotifyCountdown>,
) -> Option<(crate::model::DeviceCapabilityKind, JsonValue)> {
    // `*Remaining` sensors: live countdown in seconds, 0 if disarmed.
    if let Some(slot) = remaining_instance_to_slot(instance) {
        let seconds = countdowns
            .get(&slot)
            .map(|c| c.seconds_remaining.0.max(0))
            .unwrap_or(0);
        return Some((
            crate::model::DeviceCapabilityKind::Property,
            serde_json::json!({ "value": seconds }),
        ));
    }
    // `*Duration` Numbers: the user-set preset in minutes, 0 if disarmed.
    // Read back from the same `aa b0` slot so the Number stays in sync when
    // the user (or the Govee app) changes it elsewhere.
    if let Some(slot) = duration_instance_to_slot(instance) {
        let minutes = countdowns
            .get(&slot)
            .map(|c| (c.hh as u32) * 60 + (c.mm as u32))
            .unwrap_or(0);
        return Some((
            crate::model::DeviceCapabilityKind::Range,
            serde_json::json!({ "value": minutes }),
        ));
    }
    None
}

/// Module handle for FamilyModule registration.
pub struct Module;

impl FamilyModule for Module {
    fn supported_skus(&self) -> &'static [&'static str] {
        SUPPORTED_SKUS
    }
    fn entity_category(&self, instance: &str) -> Option<Option<String>> {
        // Countdown remaining/duration entities are secondary controls; park
        // them under HA Configuration so the device page leads with the
        // per-outlet switches.
        if owned_instance(instance) {
            Some(Some("config".to_string()))
        } else {
            None
        }
    }
    fn entity_name(&self, instance: &str) -> Option<&'static str> {
        Some(match instance {
            instance::O1_AUTO_ON_REMAINING => "Outlet 1 Auto-On Remaining",
            instance::O1_AUTO_OFF_REMAINING => "Outlet 1 Auto-Off Remaining",
            instance::O2_AUTO_ON_REMAINING => "Outlet 2 Auto-On Remaining",
            instance::O2_AUTO_OFF_REMAINING => "Outlet 2 Auto-Off Remaining",
            instance::O1_AUTO_ON_DURATION => "Outlet 1 Auto-On Duration",
            instance::O1_AUTO_OFF_DURATION => "Outlet 1 Auto-Off Duration",
            instance::O2_AUTO_ON_DURATION => "Outlet 2 Auto-On Duration",
            instance::O2_AUTO_OFF_DURATION => "Outlet 2 Auto-Off Duration",
            _ => return None,
        })
    }
    fn encode_capability(
        &self,
        sku: &str,
        instance: &str,
        value: &JsonValue,
    ) -> Option<ApiResult<Vec<String>>> {
        let (outlet, kind) = duration_instance_to_slot(instance)?;
        let minutes_total = parse_duration_minutes(value);
        let hh = (minutes_total / 60) as u8;
        let mm = (minutes_total % 60) as u8;
        log::debug!(
            "socket encode_capability {instance} value={value:?} \
             -> outlet=0x{outlet:02x} kind=0x{kind:02x} {hh}:{mm:02}"
        );
        let frames = Base64HexBytes::encode_for_sku(
            sku,
            &SetCountdown {
                outlet,
                kind,
                hh,
                mm,
            },
        );
        Some(frames.map(|b| b.base64()))
    }
    fn common_datas_seed(&self, _sku: &str, _device_id: &str) -> Option<(i32, String)> {
        None
    }
    fn status_read_frames(&self, _sku: &str) -> Vec<Vec<u8>> {
        status_read_frames()
    }
    fn keepalive_frame(&self, _sku: &str) -> Option<Vec<u8>> {
        // The app polls aa01 (outlet on/off) every ~2s to hold the link and
        // refresh state; the H5082 doesn't use the aa00 ping the projector does.
        Some(finish(vec![0xaa, 0x01]))
    }
    fn session_init_frames(
        &self,
        sku: &str,
        clock: SyncClock,
        secret: Option<[u8; 8]>,
    ) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        // The secret-key check is per-device (base64 of the cloud secret_code).
        // Without it we can't authenticate, so skip the probe rather than send a
        // wrong one; control may still work on firmware that doesn't gate on it.
        if let Some(secret) = secret {
            frames.push(check_secret_key_frame(sku, secret));
        }
        frames.push(sync_time_frame(sku, clock));
        frames
    }
}

/// 33 b2 - the SECRET_KEY_CHECK probe the app writes once per connection, right
/// after the V1 handshake (com.govee.h5080.ble.BleConstants.SINGLE_CHECK_SECRET_KEY
/// = 0xB2). The 8-byte `secret` is PER-DEVICE: base64_decode of the cloud
/// `secret_code` (SecretKeyController.q() = Encode.decryByBase64(secretCode)). The
/// device answers aa-reads without it but ignores 33-prefixed control writes until
/// the right secret arrives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CheckSecretKey {
    pub secret: SecretKeyProbe,
}

/// 33 b5 - the SYNC_TIME frame (com.govee.h5080.ble.controller.SyncTimeController).
/// The device stores timers and countdowns in local wall-clock time and has no
/// clock of its own, so it trusts the client to push the current epoch and zone
/// offset. `epoch3..epoch0` are the unix time big-endian; `offset_hours` is the
/// signed UTC offset in hours (0xf9 = -7 for US Pacific), `offset_minutes` the
/// minute part for non-whole-hour zones (30 for India UTC+5:30).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncTime {
    pub epoch3: u8,
    pub epoch2: u8,
    pub epoch1: u8,
    pub epoch0: u8,
    pub offset_hours: u8,
    pub offset_minutes: u8,
}

/// The 8-byte SECRET_KEY_CHECK payload (per-device, base64 of secret_code).
/// Defaults to zeros: a neutral placeholder for the decode path, since the real
/// value is always supplied per device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecretKeyProbe(pub [u8; 8]);

impl DecodePacketParam for SecretKeyProbe {
    fn decode_param<'a>(&mut self, data: &'a [u8]) -> anyhow::Result<&'a [u8]> {
        let bytes: [u8; 8] = data
            .get(..8)
            .ok_or_else(|| anyhow::anyhow!("EOF reading secret-key probe"))?
            .try_into()
            .expect("slice of length 8");
        *self = SecretKeyProbe(bytes);
        Ok(&data[8..])
    }
    fn encode_param(&self, target: &mut Vec<u8>) {
        target.extend_from_slice(&self.0);
    }
}

fn check_secret_key_frame(sku: &str, secret: [u8; 8]) -> Vec<u8> {
    let command = CheckSecretKey {
        secret: SecretKeyProbe(secret),
    };
    Base64HexBytes::encode_for_sku(sku, &command)
        .expect("CheckSecretKey codec is registered for socket SKUs")
        .bytes()
        .to_vec()
}

fn sync_time_frame(sku: &str, clock: SyncClock) -> Vec<u8> {
    let e = clock.epoch.to_be_bytes();
    let command = SyncTime {
        epoch3: e[0],
        epoch2: e[1],
        epoch1: e[2],
        epoch0: e[3],
        offset_hours: (clock.offset_seconds / 3600) as i8 as u8,
        offset_minutes: ((clock.offset_seconds.abs() % 3600) / 60) as u8,
    };
    Base64HexBytes::encode_for_sku(sku, &command)
        .expect("SyncTime codec is registered for socket SKUs")
        .bytes()
        .to_vec()
}

/// The aa-read status requests the app fires when it opens the H5082 page (see
/// the h5082 status-read btsnoop): the outlet on/off bitmask, then the four
/// countdown slots (each outlet x kind). The version reads (aa06/07/20/21) are
/// skipped, as for the projector. The 33b2 key-probe moved to
/// `session_init_frames`; the 33b5 time-sync write (timer accuracy only) is
/// still skipped.
fn status_read_frames() -> Vec<Vec<u8>> {
    [
        vec![0xaa, 0x01],
        vec![0xaa, 0xb0, 0x00, 0x01],
        vec![0xaa, 0xb0, 0x00, 0x00],
        vec![0xaa, 0xb0, 0x01, 0x01],
        vec![0xaa, 0xb0, 0x01, 0x00],
    ]
    .into_iter()
    .map(finish)
    .collect()
}

/// 33 b0 - arm or disarm a countdown ("auto-on" / "auto-off" in the app).
/// `outlet` is the wire byte (0x01 outlet 1, 0x00 outlet 2). `kind` is
/// 0x01 fire-ON, 0x00 fire-OFF. `hh:mm` is the countdown duration; setting
/// both to 0 disarms the slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetCountdown {
    pub outlet: u8,
    pub kind: u8,
    pub hh: u8,
    pub mm: u8,
}

/// aa b0 - countdown read. `seconds_remaining` is the live countdown the
/// device decrements at wall-clock rate (per TurnOnOffDelayController.java
/// in the decompiled app). Disarmed slots report `hh:mm` and
/// `seconds_remaining` all zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotifyCountdown {
    pub outlet: u8,
    pub kind: u8,
    pub hh: u8,
    pub mm: u8,
    pub seconds_remaining: SecondsRemainingBe24,
}

/// 24-bit big-endian signed integer carrying the seconds-remaining field of
/// an `aa b0` countdown read. The wire layout the app parses is three bytes
/// sign-extended to i32 (`getSignedInt(bArr, true)`); empty slots zero the
/// field and any negative value would be a wire error.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecondsRemainingBe24(pub i32);

impl DecodePacketParam for SecondsRemainingBe24 {
    fn decode_param<'a>(&mut self, data: &'a [u8]) -> anyhow::Result<&'a [u8]> {
        let bytes: [u8; 3] = data
            .get(..3)
            .ok_or_else(|| anyhow::anyhow!("EOF reading BE24"))?
            .try_into()
            .expect("slice of length 3");
        let raw = ((bytes[0] as i32) << 16) | ((bytes[1] as i32) << 8) | (bytes[2] as i32);
        // sign-extend bit 23 to i32
        let value = if raw & (1 << 23) != 0 {
            raw | !0xFF_FFFFi32
        } else {
            raw
        };
        *self = SecondsRemainingBe24(value);
        Ok(&data[3..])
    }
    fn encode_param(&self, target: &mut Vec<u8>) {
        let v = self.0;
        target.push(((v >> 16) & 0xFF) as u8);
        target.push(((v >> 8) & 0xFF) as u8);
        target.push((v & 0xFF) as u8);
    }
}

/// 33 13 - set or update a recurring timer slot. `flags` packs bit7 = enabled
/// and bit0 = fire-type (1 = on, 0 = off). `days` is a Mon=bit0..Sun=bit6
/// mask with bit7 = "selective" flag, except for the special `0x00` value
/// which is the every-day sentinel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetTimerSlot {
    pub outlet: u8,
    pub slot: u8,
    pub flags: u8,
    pub hh: u8,
    pub mm: u8,
    pub days: u8,
}

/// aa 12 - per-outlet timer count. The H5082 cloud status broadcast emits
/// one of these per outlet ahead of the `aa b4` array snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotifyTimerCount {
    pub outlet: u8,
    pub count: u8,
}

/// aa 01 - outlet on/off bitmask read (and the app's keepalive poll). `bits`
/// is the same packing as the IoT onOff value: bit0 = outlet 2, bit1 = outlet 1
/// (`03` = both on, `01` = outlet 2 only, `00` = both off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotifyOutletState {
    pub bits: u8,
}

/// 33 01 <packed> - per-outlet power write. The byte is `(mask << 4) | on_bits`:
/// the high nibble selects which outlets the command addresses (bit1 = outlet 1,
/// bit0 = outlet 2), the low nibble is which of those to switch on. The app
/// sends this same frame over BLE and (base64-wrapped) over IoT, e.g. `33 01 22`
/// = outlet 1 on, `33 01 10` = outlet 2 off, `33 01 33` = both on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetSocketPower {
    pub packed: u8,
}

impl SetSocketPower {
    /// Pack an outlet `mask` (which outlets to address) and `on_bits` (which of
    /// them to turn on) into the single wire byte.
    pub fn new(mask: u8, on_bits: u8) -> Self {
        Self {
            packed: (mask << 4) | (on_bits & 0x0f),
        }
    }
}

pub(super) fn register(codecs: &mut Vec<PacketCodec>) {
    // 33 b0 <outlet> <kind> <hh> <mm> - arm/disarm countdown.
    codecs.push(packet!(
        SUPPORTED_SKUS,
        SetCountdown,
        SetCountdown,
        0x33,
        0xB0,
        outlet,
        kind,
        hh,
        mm,
    ));

    // aa b0 <outlet> <kind> <hh> <mm> <seconds_remaining:BE24> - countdown read.
    codecs.push(packet!(
        SUPPORTED_SKUS,
        NotifyCountdown,
        NotifyCountdown,
        0xAA,
        0xB0,
        outlet,
        kind,
        hh,
        mm,
        seconds_remaining,
    ));

    // 33 13 <outlet> <slot> <flags> <hh> <mm> <days> 00 - set timer slot.
    codecs.push(packet!(
        SUPPORTED_SKUS,
        SetTimerSlot,
        SetTimerSlot,
        0x33,
        0x13,
        outlet,
        slot,
        flags,
        hh,
        mm,
        days,
        0x00,
    ));

    // aa 12 <outlet> <count> - per-outlet timer count read.
    codecs.push(packet!(
        SUPPORTED_SKUS,
        NotifyTimerCount,
        NotifyTimerCount,
        0xAA,
        0x12,
        outlet,
        count,
    ));

    // aa 01 <bits> - outlet on/off bitmask read (and the keepalive poll).
    codecs.push(packet!(
        SUPPORTED_SKUS,
        NotifyOutletState,
        NotifyOutletState,
        0xAA,
        0x01,
        bits,
    ));

    // 33 01 <packed> - per-outlet power write.
    codecs.push(packet!(
        SUPPORTED_SKUS,
        SetSocketPower,
        SetSocketPower,
        0x33,
        0x01,
        packed,
    ));

    // 33 b2 <8-byte fixed secret> - post-handshake SECRET_KEY_CHECK auth probe.
    codecs.push(packet!(
        SUPPORTED_SKUS,
        CheckSecretKey,
        CheckSecretKey,
        0x33,
        0xB2,
        secret,
    ));

    // 33 b5 <epoch:BE32> 01 <offset_hours> <offset_minutes> - clock sync.
    codecs.push(packet!(
        SUPPORTED_SKUS,
        SyncTime,
        SyncTime,
        0x33,
        0xB5,
        epoch3,
        epoch2,
        epoch1,
        epoch0,
        0x01,
        offset_hours,
        offset_minutes,
    ));
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ble::Base64HexBytes;
    use crate::ble::codec::GoveeBlePacket;

    fn enc<T: 'static>(value: &T) -> Vec<u8> {
        Base64HexBytes::encode_for_sku("H5082", value)
            .unwrap()
            .bytes()
            .to_vec()
    }

    #[test]
    fn secret_key_check_matches_capture() {
        // The app's post-handshake SECRET_KEY_CHECK probe for the owned device:
        // research/mitm "Untitled - (null) (6).log" #1930 decrypts to
        // 33 b2 3f d6 59 2f 5b 3d d1 07 (then zero-pad to 19 + xor checksum 0xae).
        // The 8-byte secret is per-device (base64 of secret_code) so the test
        // passes it explicitly.
        let secret = [0x3f, 0xd6, 0x59, 0x2f, 0x5b, 0x3d, 0xd1, 0x07];
        let mut expect = vec![0x33, 0xb2];
        expect.extend_from_slice(&secret);
        expect.resize(19, 0);
        expect.push(expect.iter().fold(0u8, |a, b| a ^ b));
        assert_eq!(check_secret_key_frame("H5082", secret), expect);
        assert_eq!(expect[19], 0xae);
    }

    #[test]
    fn sync_time_matches_capture() {
        // From the same capture #2023: app's SYNC_TIME on a Pacific (UTC-7) phone
        // decrypts to 33 b5 6a 1a 82 8b 01 f9 00 (epoch 0x6a1a828b, marker 01,
        // offset hours 0xf9 = -7, minutes 00; xor checksum 0x07).
        let frame = sync_time_frame(
            "H5082",
            SyncClock {
                epoch: 0x6a1a_828b,
                offset_seconds: -7 * 3600,
            },
        );
        let mut expect = vec![0x33, 0xb5, 0x6a, 0x1a, 0x82, 0x8b, 0x01, 0xf9, 0x00];
        expect.resize(19, 0);
        expect.push(expect.iter().fold(0u8, |a, b| a ^ b));
        assert_eq!(frame, expect);
        assert_eq!(expect[19], 0x07);
    }

    #[test]
    fn sync_time_half_hour_zone() {
        // India Standard Time, UTC+5:30: hours 5, minutes 30 (0x1e).
        let frame = sync_time_frame(
            "H5082",
            SyncClock {
                epoch: 0,
                offset_seconds: 5 * 3600 + 30 * 60,
            },
        );
        assert_eq!(&frame[6..9], &[0x01, 0x05, 0x1e]);
    }

    #[test]
    fn init_frames_annotate_from_codec() {
        use crate::ble::codec::FieldRole;

        let probe = check_secret_key_frame("H5082", [1, 2, 3, 4, 5, 6, 7, 8]);
        let ann = crate::ble::annotate_frame("H5082", &probe, true);
        assert_eq!(ann.summary, "secret-key check");
        assert_eq!(ann.fields[1].role, FieldRole::Opcode);
        assert_eq!(ann.fields[2].label, "secret");
        assert_eq!(ann.fields[2].role, FieldRole::Field);

        let clock = sync_time_frame(
            "H5082",
            SyncClock {
                epoch: 0x6a1a_828b,
                offset_seconds: -7 * 3600,
            },
        );
        let ann = crate::ble::annotate_frame("H5082", &clock, true);
        assert_eq!(ann.summary, "sync time");
        assert_eq!(ann.fields[2].label, "epoch3");
        assert_eq!(ann.fields[5].label, "epoch0");
        assert_eq!(ann.fields[6].role, FieldRole::Const);
        assert_eq!(ann.fields[7].label, "offset_hours");
        assert_eq!(ann.fields[8].label, "offset_minutes");
    }

    #[test]
    fn socket_power_summary_decodes_outlets() {
        // 33 01 22 = address outlet bit1, turn it on (outbound control write)
        let ann = crate::ble::annotate_frame("H5082", &enc(&SetSocketPower::new(2, 2)), true);
        assert_eq!(ann.summary, "set outlet power (outlet1=on)");

        // 33 01 30 = address both outlets, turn both off
        let ann = crate::ble::annotate_frame("H5082", &enc(&SetSocketPower::new(3, 0)), true);
        assert_eq!(ann.summary, "set outlet power (outlet0=off outlet1=off)");

        // aa 01 03 = both outlets reported on (inbound reply, not a read request)
        let ann =
            crate::ble::annotate_frame("H5082", &enc(&NotifyOutletState { bits: 0x03 }), false);
        assert_eq!(ann.summary, "outlet state (outlet0,outlet1 on)");

        // aa 01 00 outbound is the keepalive read REQUEST, not "all off"
        let ann = crate::ble::annotate_frame("H5082", &enc(&NotifyOutletState { bits: 0 }), true);
        assert_eq!(ann.summary, "read outlet state");
    }

    #[test]
    fn countdown_set_matches_capture() {
        // From research/mitm/h5082-full.btsnoop t=119.0: outlet 1 fire-ON 18h 50m.
        // Wire frame: 33 b0 01 01 12 32 (then zero-padded to 19 + checksum).
        let mut expect = vec![0x33, 0xB0, 0x01, 0x01, 0x12, 0x32];
        expect.resize(19, 0);
        // XOR checksum of the first 19 bytes
        expect.push(expect.iter().fold(0u8, |a, b| a ^ b));
        assert_eq!(
            enc(&SetCountdown {
                outlet: 0x01,
                kind: 0x01,
                hh: 18,
                mm: 50,
            }),
            expect
        );
    }

    #[test]
    fn countdown_disarm_is_zero_duration() {
        // 0:00 is the disable sentinel; outlet 1 fire-OFF at t=166.2.
        let mut expect = vec![0x33, 0xB0, 0x01, 0x00, 0x00, 0x00];
        expect.resize(19, 0);
        expect.push(expect.iter().fold(0u8, |a, b| a ^ b));
        assert_eq!(
            enc(&SetCountdown {
                outlet: 0x01,
                kind: 0x00,
                hh: 0,
                mm: 0,
            }),
            expect
        );
    }

    #[test]
    fn countdown_read_decodes_armed_slot() {
        // From research/mitm/h5082-followup.btsnoop t=3.78: outlet 2 fire-ON,
        // preset 15h 0m, 53388 seconds remaining (BE24 = 00 d0 8c).
        let bytes =
            Base64HexBytes::with_bytes(vec![0xAA, 0xB0, 0x00, 0x01, 0x0F, 0x00, 0x00, 0xD0, 0x8C]);
        assert_eq!(
            bytes.decode_for_sku("H5082"),
            GoveeBlePacket::NotifyCountdown(NotifyCountdown {
                outlet: 0x00,
                kind: 0x01,
                hh: 0x0F,
                mm: 0x00,
                seconds_remaining: SecondsRemainingBe24(53388),
            })
        );
    }

    #[test]
    fn countdown_read_decodes_empty_slot() {
        // From research/iot-trace.log 2026-05-28: outlet 2 fire-ON, all zero.
        let bytes =
            Base64HexBytes::with_bytes(vec![0xAA, 0xB0, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(
            bytes.decode_for_sku("H5082"),
            GoveeBlePacket::NotifyCountdown(NotifyCountdown {
                outlet: 0x00,
                kind: 0x01,
                hh: 0,
                mm: 0,
                seconds_remaining: SecondsRemainingBe24(0),
            })
        );
    }

    #[test]
    fn timer_set_matches_capture_tue_fri() {
        // From research/mitm/h5082-full.btsnoop t=222.9:
        // outlet 1, slot 0, enabled + fire-off, 17:11, Tue+Fri.
        // Wire: 33 13 01 00 80 11 0b 92 00
        let mut expect = vec![0x33, 0x13, 0x01, 0x00, 0x80, 0x11, 0x0B, 0x92, 0x00];
        expect.resize(19, 0);
        expect.push(expect.iter().fold(0u8, |a, b| a ^ b));
        assert_eq!(
            enc(&SetTimerSlot {
                outlet: 0x01,
                slot: 0,
                flags: 0x80,
                hh: 17,
                mm: 11,
                days: 0x92,
            }),
            expect
        );
    }

    #[test]
    fn timer_set_matches_capture_every_day_except_thu() {
        // From research/mitm/h5082-followup.btsnoop t=112.4:
        // outlet 1, slot 1, enabled + fire-on, 13:10, every day except Thursday.
        // Wire: 33 13 01 01 81 0d 0a f7 00. Anchors the Mon=bit0..Sun=bit6 ordering.
        let mut expect = vec![0x33, 0x13, 0x01, 0x01, 0x81, 0x0D, 0x0A, 0xF7, 0x00];
        expect.resize(19, 0);
        expect.push(expect.iter().fold(0u8, |a, b| a ^ b));
        assert_eq!(
            enc(&SetTimerSlot {
                outlet: 0x01,
                slot: 1,
                flags: 0x81,
                hh: 13,
                mm: 10,
                days: 0xF7,
            }),
            expect
        );
    }

    #[test]
    fn timer_count_decodes() {
        // From research/iot-trace.log: outlet 1 has 0 timers configured.
        let bytes = Base64HexBytes::with_bytes(vec![0xAA, 0x12, 0x01, 0x00]);
        assert_eq!(
            bytes.decode_for_sku("H5082"),
            GoveeBlePacket::NotifyTimerCount(NotifyTimerCount {
                outlet: 0x01,
                count: 0,
            })
        );
    }

    #[test]
    fn encode_capability_arm_outlet1_auto_on_18h50m() {
        // 18*60+50 = 1130 minutes → SetCountdown { outlet: 0x01, kind: 0x01, hh: 18, mm: 50 }
        // → wire frame 33 b0 01 01 12 32 (same bytes the BLE capture showed at
        // research/mitm/h5082-full.btsnoop t=119.0).
        use crate::ble::family::FamilyModule;
        let frames = Module
            .encode_capability(
                "H5082",
                instance::O1_AUTO_ON_DURATION,
                &serde_json::json!(1130),
            )
            .expect("instance owned")
            .expect("encodes");
        let bytes = data_encoding::BASE64.decode(frames[0].as_bytes()).unwrap();
        assert_eq!(&bytes[..6], &[0x33, 0xB0, 0x01, 0x01, 0x12, 0x32]);
    }

    #[test]
    fn encode_capability_disarm_via_zero() {
        // 0 minutes is the disarm sentinel: 33 b0 <outlet> <kind> 00 00.
        use crate::ble::family::FamilyModule;
        let frames = Module
            .encode_capability(
                "H5082",
                instance::O2_AUTO_OFF_DURATION,
                &serde_json::json!(0),
            )
            .expect("instance owned")
            .expect("encodes");
        let bytes = data_encoding::BASE64.decode(frames[0].as_bytes()).unwrap();
        assert_eq!(&bytes[..6], &[0x33, 0xB0, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn parse_timer_request_matches_capture_tue_fri() {
        // Reconstruct outlet 1, slot 0, off, 17:11, Tue+Fri from the
        // h5082-full.btsnoop capture (wire `33 13 01 00 80 11 0b 92 00`).
        let req = parse_timer_request(&serde_json::json!({
            "outlet": 1,
            "slot": 0,
            "kind": "off",
            "time": "17:11",
            "days": ["tue", "fri"],
        }))
        .unwrap();
        assert_eq!(
            req,
            SetTimerSlot {
                outlet: 0x01,
                slot: 0,
                flags: 0x80,
                hh: 17,
                mm: 11,
                days: 0x92,
            }
        );
    }

    #[test]
    fn parse_timer_request_every_day_sentinel() {
        // Empty array AND "all" both produce the 0x00 every-day sentinel
        // the device expects.
        for days_value in [serde_json::json!([]), serde_json::json!("all")] {
            let req = parse_timer_request(&serde_json::json!({
                "outlet": 1,
                "slot": 1,
                "kind": "on",
                "time": "13:10",
                "days": days_value,
            }))
            .unwrap();
            assert_eq!(req.days, 0x00);
            assert_eq!(req.flags, 0x81);
        }
    }

    #[test]
    fn parse_timer_request_disabled_clears_enable_bit() {
        let req = parse_timer_request(&serde_json::json!({
            "outlet": 2,
            "slot": 0,
            "kind": "off",
            "time": "00:00",
            "days": [],
            "enabled": false,
        }))
        .unwrap();
        // bit 7 = 0 (disabled), bit 0 = 0 (off-kind)
        assert_eq!(req.flags, 0x00);
    }

    #[test]
    fn parse_timer_request_returns_specific_errors() {
        let bad_outlet = parse_timer_request(&serde_json::json!({
            "outlet": 5,
            "slot": 0,
            "kind": "on",
            "time": "0:00",
            "days": "all",
        }));
        assert_eq!(bad_outlet, Err(TimerParseError::BadOutlet));

        let bad_time = parse_timer_request(&serde_json::json!({
            "outlet": 1,
            "slot": 0,
            "kind": "on",
            "time": "25:00",
            "days": "all",
        }));
        assert_eq!(bad_time, Err(TimerParseError::BadTime));

        let bad_day = parse_timer_request(&serde_json::json!({
            "outlet": 1,
            "slot": 0,
            "kind": "on",
            "time": "0:00",
            "days": ["funday"],
        }));
        assert_eq!(bad_day, Err(TimerParseError::BadDays));
    }

    #[test]
    fn parse_duration_accepts_number_and_string() {
        // Numeric payload (the typical HA path)
        assert_eq!(parse_duration_minutes(&serde_json::json!(30)), 30);
        // Float payload
        assert_eq!(parse_duration_minutes(&serde_json::json!(30.4)), 30);
        // String payload (some MQTT discovery clients quote)
        assert_eq!(parse_duration_minutes(&serde_json::json!("30")), 30);
        // Clamped to the 23:59 ceiling
        assert_eq!(
            parse_duration_minutes(&serde_json::json!(99_999)),
            23 * 60 + 59
        );
        // Unparseable falls through to 0
        assert_eq!(parse_duration_minutes(&serde_json::json!("nope")), 0);
        assert_eq!(parse_duration_minutes(&serde_json::json!(null)), 0);
    }

    #[test]
    fn record_optimistic_write_round_trips() {
        let c =
            record_optimistic_write(instance::O1_AUTO_OFF_DURATION, &serde_json::json!(1)).unwrap();
        assert_eq!(c.outlet, 0x01);
        assert_eq!(c.kind, 0x00);
        assert_eq!(c.hh, 0);
        assert_eq!(c.mm, 1);
        assert!(record_optimistic_write("powerSwitch", &serde_json::json!(1)).is_none());
    }

    #[test]
    fn encode_capability_unknown_instance_is_none() {
        use crate::ble::family::FamilyModule;
        assert!(
            Module
                .encode_capability("H5082", "noSuchInstance", &serde_json::json!(1))
                .is_none()
        );
    }

    #[test]
    fn state_value_remaining_reflects_held_countdown() {
        let mut state = std::collections::HashMap::new();
        state.insert(
            (0x00, 0x01),
            NotifyCountdown {
                outlet: 0x00,
                kind: 0x01,
                hh: 15,
                mm: 0,
                seconds_remaining: SecondsRemainingBe24(53388),
            },
        );
        // Outlet 2 auto-on remaining (Property = read-only sensor)
        let (kind, value) = state_value(instance::O2_AUTO_ON_REMAINING, &state).unwrap();
        assert_eq!(kind, crate::model::DeviceCapabilityKind::Property);
        assert_eq!(value, serde_json::json!({ "value": 53388 }));
        // Same slot's preset duration in minutes (Range = editable Number)
        let (kind, value) = state_value(instance::O2_AUTO_ON_DURATION, &state).unwrap();
        assert_eq!(kind, crate::model::DeviceCapabilityKind::Range);
        assert_eq!(value, serde_json::json!({ "value": 15 * 60 }));
    }

    #[test]
    fn seconds_remaining_be24_sign_extends() {
        let mut s = SecondsRemainingBe24::default();
        // 00 d0 8c = 53388, positive
        s.decode_param(&[0x00, 0xD0, 0x8C]).unwrap();
        assert_eq!(s, SecondsRemainingBe24(53388));
        // ff ff ff = -1 sign-extended
        s.decode_param(&[0xFF, 0xFF, 0xFF]).unwrap();
        assert_eq!(s, SecondsRemainingBe24(-1));
        // round-trip
        let mut buf = vec![];
        SecondsRemainingBe24(53388).encode_param(&mut buf);
        assert_eq!(buf, vec![0x00, 0xD0, 0x8C]);
    }

    #[test]
    fn aa01_outlet_state_decodes() {
        // captured outlet bitmask reads: both on / outlet 2 only / both off
        for (byte, bits) in [(0x03, 0x03), (0x01, 0x01), (0x00, 0x00)] {
            let frame = Base64HexBytes::with_bytes(vec![0xAA, 0x01, byte]);
            assert_eq!(
                frame.decode_for_sku("H5082"),
                GoveeBlePacket::NotifyOutletState(NotifyOutletState { bits })
            );
        }
    }

    #[test]
    fn sku_ble_support_gates_on_codecs() {
        // a family owns the SKU -> we have BLE codecs for it
        assert!(crate::ble::sku_has_ble_support("H5082")); // socket family
        assert!(crate::ble::sku_has_ble_support("H6093")); // projector family
        // no family -> no codecs -> not BLE-controllable despite a BLE address
        assert!(!crate::ble::sku_has_ble_support("H5083"));
    }

    #[test]
    fn set_socket_power_matches_capture() {
        // (mask, on_bits, packed byte) from the h5082 control btsnoop:
        // left=outlet1 (bit1), right=outlet2 (bit0), master=both.
        for (mask, on, packed) in [
            (2, 2, 0x22), // left on
            (2, 0, 0x20), // left off
            (1, 1, 0x11), // right on
            (1, 0, 0x10), // right off
            (3, 3, 0x33), // master on
            (3, 0, 0x30), // master off
        ] {
            let frame = enc(&SetSocketPower::new(mask, on));
            assert_eq!(&frame[..3], &[0x33, 0x01, packed]);
        }
    }

    #[test]
    fn status_read_and_keepalive_for_h5082() {
        // the page-open read burst: aa01 then the four aab0 countdown slots
        let frames = crate::ble::status_read_frames("H5082");
        assert_eq!(frames.len(), 5);
        assert_eq!(&frames[0][..2], &[0xaa, 0x01]);
        assert_eq!(&frames[1][..4], &[0xaa, 0xb0, 0x00, 0x01]);
        assert_eq!(&frames[4][..4], &[0xaa, 0xb0, 0x01, 0x00]);
        // the H5082 polls aa01 to hold the link, not the generic aa00
        let keepalive = crate::ble::keepalive_frame("H5082").expect("h5082 overrides keepalive");
        assert_eq!(&keepalive[..2], &[0xaa, 0x01]);
    }
}
