//! BLE frames common to every Govee device, not tied to a single device family.
//!
//! These are protocol-level rather than family-specific: the aa 00 keepalive the
//! status reader exchanges to hold a link open, and the ee 30 power-state notify
//! a device pushes when its power changes (e.g. a physical button press). They
//! are registered under the [`ALL_SKUS`](super::codec::ALL_SKUS) wildcard so they
//! decode for any device, rather than being attached to one family's module.

use super::codec::FieldSpec::{Const, Field};
use super::codec::{ALL_SKUS, CodecUnsupported, GoveeBlePacket, PacketCodec, finish};
use serde::Serialize;

/// ee 30 01 - device-pushed power state notification, sent unsolicited when the
/// power changes (including a physical button press). Notify-only; we never send
/// it. byte3 is the new state, byte4 mirrors it (01/01 on, 00/00 off).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
pub struct NotifyPower {
    pub on: bool,
}

/// aa 00 - the keepalive ping. The status reader sends it on a timer to hold the
/// link open; the device echoes the same frame back. No payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
pub struct NotifyKeepalive;

pub(super) fn register(codecs: &mut Vec<PacketCodec>) {
    // ee 30 01 - device-pushed power state. Notify-only, so it is hand-written
    // (no encode) and tolerant of the non-zero mirror byte the packet! macro's
    // zero-padding check would reject. Field specs make it annotate.
    codecs.push(
        PacketCodec::new(
            &[ALL_SKUS],
            |_: &NotifyPower| Err(CodecUnsupported("NotifyPower is notify-only").into()),
            |data: &[u8]| {
                anyhow::ensure!(
                    data.len() >= 4 && data[0] == 0xee && data[1] == 0x30 && data[2] == 0x01,
                    "not an ee 30 01 power notify"
                );
                Ok(GoveeBlePacket::NotifyPower(NotifyPower {
                    on: data[3] != 0,
                }))
            },
        )
        .with_field_specs(vec![
            Const(0xee),
            Const(0x30),
            Const(0x01),
            Field("on"),
            Field("confirm"),
        ]),
    );

    // aa 00 - keepalive ping/echo, no payload. Decoding it lets the inspector
    // label both directions "keepalive" instead of "undecoded".
    codecs.push(
        PacketCodec::new(
            &[ALL_SKUS],
            |_: &NotifyKeepalive| Ok(finish(vec![0xaa, 0x00])),
            |data: &[u8]| {
                anyhow::ensure!(
                    data.len() >= 2 && data[0] == 0xaa && data[1] == 0x00,
                    "not an aa 00 keepalive"
                );
                Ok(GoveeBlePacket::NotifyKeepalive(NotifyKeepalive))
            },
        )
        .with_field_specs(vec![Const(0xaa), Const(0x00)]),
    );
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn ee30_power_notify_decodes_and_annotates() {
        // physical power button: on / off (captured from the H6093)
        let on = [
            0xee, 0x30, 0x01, 0x01, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xdf,
        ];
        let off = [
            0xee, 0x30, 0x01, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xdf,
        ];
        assert_eq!(
            crate::ble::decode_frame("H6093", &on),
            GoveeBlePacket::NotifyPower(NotifyPower { on: true })
        );
        assert_eq!(
            crate::ble::decode_frame("H6093", &off),
            GoveeBlePacket::NotifyPower(NotifyPower { on: false })
        );
        let ann = crate::ble::annotate_frame("H6093", &on, false);
        assert_eq!(ann.summary, "power state notify");
        assert_eq!(ann.fields[3].label, "on");
    }

    #[test]
    fn keepalive_decodes_for_any_sku() {
        let frame = [
            0xaa, 0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xaa,
        ];
        // wildcard registration: decodes for a real family SKU and for one with
        // no family codecs at all, proving it isn't tied to the projector.
        for sku in ["H6093", "NONEXISTENT"] {
            assert_eq!(
                crate::ble::decode_frame(sku, &frame),
                GoveeBlePacket::NotifyKeepalive(NotifyKeepalive)
            );
        }
        assert_eq!(
            crate::ble::annotate_frame("NONEXISTENT", &frame, false).summary,
            "keepalive"
        );
    }
}
