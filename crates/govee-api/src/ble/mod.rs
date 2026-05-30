//! Govee BLE-frame command encoding/decoding, split by device family.
//!
//! [`codec`] holds the frame engine (the 20-byte `0x33`/`0xaa` framing, the
//! per-SKU codec registry, and the `packet!` macro). Each sibling module owns
//! one device family's command structs and registers its codecs with the
//! manager. To support a new family, add a module with a `register` fn and call
//! it from [`codec::PacketManager::new`].

pub mod codec;
pub mod common;
pub mod encryption;
pub mod family;
pub mod humidifier;
pub mod light;
pub mod projector;
pub mod socket;

use once_cell::sync::Lazy;

static MGR: Lazy<codec::PacketManager> = Lazy::new(codec::PacketManager::new);

/// Decode a raw device frame (the 20 bytes as received on the wire, checksum
/// included) into a typed packet for the given SKU. Returns
/// [`GoveeBlePacket::Generic`] when no registered codec matches. Used by the
/// direct-BLE reader to fold inbound aa-status notifications into held state.
pub fn decode_frame(sku: &str, frame: &[u8]) -> GoveeBlePacket {
    MGR.decode_for_sku(sku, frame)
}

/// Annotate a raw frame for the inspector: a one-line summary plus a per-byte
/// field map, sourced from the codec that decodes it (so field names double as
/// the inspector's per-byte docs). Structural-only for frames no codec matches.
pub fn annotate_frame(sku: &str, frame: &[u8]) -> codec::FrameAnnotation {
    MGR.annotate_for_sku(sku, frame)
}

// Re-export the public surface flat off `ble::` so callers import
// `ble::Base64HexBytes` etc. rather than reaching into the family modules. The
// codec engine types (PacketManager, PacketCodec, ...) stay in `ble::codec`;
// nothing outside this module uses them directly.
pub use codec::{Base64HexBytes, FieldNote, FieldRole, FrameAnnotation, GoveeBlePacket};
pub use common::{NotifyKeepalive, NotifyPower};
pub use family::{
    common_datas_seed, encode_capability, entity_category, entity_name, status_read_frames,
};
pub use humidifier::{
    HumidifierAutoMode, NotifyHumidifierMode, NotifyHumidifierNightlightParams, SetHumidifierMode,
    SetHumidifierNightlightParams, TargetHumidity,
};
pub use light::{SetBrightness, SetDevicePower, SetSceneCode};
// State-mutation helpers that take H6093-specific structs stay as direct
// `projector_*` calls in src/; the FamilyModule trait owns only the
// SKU-agnostic surface.
pub use projector::{
    AuroraColorMode, NotifyAurora, NotifyLaser, ProjectorSettings, SetAurora, SetAuroraLaser,
    SetAutoOff, SetDreamViewLaser, SetPairingSound, SetPairingStatus, SetSilentPowerUp,
    apply_auto_off_field as projector_apply_auto_off_field,
    apply_blob_field as projector_apply_blob_field, state_value as projector_state_value,
};

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn packet_manager() {
        assert_eq!(
            MGR.decode_for_sku(
                "H7160",
                &[
                    0x33, 0x05, 0x01, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 23
                ]
            ),
            GoveeBlePacket::SetHumidifierMode(SetHumidifierMode {
                mode: 1,
                param: 0x20
            })
        );

        assert_eq!(
            MGR.encode_for_sku(
                "H7160",
                &SetHumidifierMode {
                    mode: 1,
                    param: 0x20
                }
            )
            .unwrap(),
            vec![
                0x33, 0x05, 0x01, 0x20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 23
            ]
        );
    }

    fn round_trip<T: 'static + std::fmt::Debug>(sku: &str, value: &T, expect: GoveeBlePacket) {
        let bytes = Base64HexBytes::encode_for_sku(sku, value).unwrap();
        let decoded = bytes.decode_for_sku(sku);
        assert_eq!(decoded, expect);
    }

    #[test]
    fn basic_round_trip() {
        round_trip(
            "Generic:Light",
            &SetDevicePower { on: true },
            GoveeBlePacket::SetDevicePower(SetDevicePower { on: true }),
        );
        round_trip(
            "H7160",
            &SetHumidifierNightlightParams {
                on: true,
                r: 255,
                g: 69,
                b: 42,
                brightness: 100,
            },
            GoveeBlePacket::SetHumidifierNightlight(SetHumidifierNightlightParams {
                on: true,
                r: 255,
                g: 69,
                b: 42,
                brightness: 100,
            }),
        );
    }
}
