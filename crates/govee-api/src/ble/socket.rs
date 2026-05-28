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

use super::codec::{DecodePacketParam, PacketCodec};
use super::family::FamilyModule;
use crate::error::ApiResult;
use crate::packet;
use serde_json::Value as JsonValue;

const SUPPORTED_SKUS: &[&str] = &["H5082"];

/// User-facing instance names. Outlet numbers in the names are 1-based to match
/// the app and the existing per-outlet switches; the wire byte is the inverse
/// (outlet 1 = 0x01, outlet 2 = 0x00) and is translated at the codec boundary.
pub mod instance {
    /// Live seconds remaining on outlet 1's fire-ON countdown, 0 when disarmed.
    pub const O1_AUTO_ON_REMAINING: &str = "outlet1AutoOnRemaining";
    pub const O1_AUTO_OFF_REMAINING: &str = "outlet1AutoOffRemaining";
    pub const O2_AUTO_ON_REMAINING: &str = "outlet2AutoOnRemaining";
    pub const O2_AUTO_OFF_REMAINING: &str = "outlet2AutoOffRemaining";
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

/// Map a synthesized capability instance name to its `(outlet_wire, kind_wire)`
/// pair. `kind_wire` is the same value the device uses on the wire
/// (`0x00` = fire-OFF, `0x01` = fire-ON).
pub fn instance_to_slot(instance: &str) -> Option<(u8, u8)> {
    Some(match instance {
        instance::O1_AUTO_ON_REMAINING => (0x01, 0x01),
        instance::O1_AUTO_OFF_REMAINING => (0x01, 0x00),
        instance::O2_AUTO_ON_REMAINING => (0x00, 0x01),
        instance::O2_AUTO_OFF_REMAINING => (0x00, 0x00),
        _ => return None,
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
    let slot = instance_to_slot(instance)?;
    let seconds = countdowns
        .get(&slot)
        .map(|c| c.seconds_remaining.0.max(0))
        .unwrap_or(0);
    Some((
        crate::model::DeviceCapabilityKind::Range,
        serde_json::json!({ "value": seconds }),
    ))
}

/// Module handle for FamilyModule registration.
pub struct Module;

impl FamilyModule for Module {
    fn supported_skus(&self) -> &'static [&'static str] {
        SUPPORTED_SKUS
    }
    fn entity_category(&self, instance: &str) -> Option<Option<String>> {
        // Remaining-seconds sensors are diagnostic-flavored, parked under HA
        // Configuration so the device's main page stays focused on the
        // per-outlet switches.
        if instance_to_slot(instance).is_some() {
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
            _ => return None,
        })
    }
    fn encode_capability(
        &self,
        _sku: &str,
        _instance: &str,
        _value: &JsonValue,
    ) -> Option<ApiResult<Vec<String>>> {
        // Read-only sensors; the write path lands in a follow-up commit.
        None
    }
    fn common_datas_seed(&self, _sku: &str, _device_id: &str) -> Option<(i32, String)> {
        None
    }
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
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ble::Base64HexBytes;
    use crate::ble::codec::GoveeBlePacket;

    fn enc<T: 'static>(value: &T) -> Vec<u8> {
        Base64HexBytes::encode_for_sku("H5082", value).unwrap().bytes().to_vec()
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
        let bytes = Base64HexBytes::with_bytes(vec![
            0xAA, 0xB0, 0x00, 0x01, 0x0F, 0x00, 0x00, 0xD0, 0x8C,
        ]);
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
        let bytes = Base64HexBytes::with_bytes(vec![0xAA, 0xB0, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]);
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
}
