//! The BLE-frame codec engine, shared across all device families.
//!
//! Govee devices speak a 20-byte BLE frame format (`0x33` write / `0xaa` read,
//! XOR checksum in the last byte). This
//! module holds the machinery that encodes and decodes those frames and the
//! per-SKU codec registry; the actual per-device command structs and their
//! registrations live in the sibling family modules (humidifier, light, ...).

use crate::error::{ApiResult, GoveeApiError};
use anyhow::anyhow;
use parking_lot::{MappedMutexGuard, Mutex, MutexGuard};
use serde::{Deserialize, Deserializer};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// Marker error returned by codec closures that intentionally do not
/// implement an operation (currently: decoders for write-only command
/// frames). The codec boundary downcasts on this so the resulting public
/// `GoveeApiError` is `Unsupported` rather than `Protocol`. The closure
/// signature stays `anyhow::Result` so the rest of the codec machinery
/// is undisturbed; this is the one error category that needs to round-trip
/// through anyhow without losing its identity.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct CodecUnsupported(pub(crate) &'static str);

#[derive(Clone, PartialEq, Eq)]
pub struct HexBytes(pub(crate) Vec<u8>);

impl std::fmt::Debug for HexBytes {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.write_fmt(format_args!("{:02X?}", self.0))
    }
}

#[allow(clippy::type_complexity)]
pub struct PacketCodec {
    encode: Box<dyn Fn(&dyn Any) -> anyhow::Result<Vec<u8>> + Sync + Send>,
    decode: Box<dyn Fn(&[u8]) -> anyhow::Result<GoveeBlePacket> + Sync + Send>,
    supported_skus: &'static [&'static str],
    type_id: TypeId,
}

impl PacketCodec {
    pub fn new<T: 'static>(
        supported_skus: &'static [&'static str],
        encode: impl Fn(&T) -> anyhow::Result<Vec<u8>> + 'static + Sync + Send,
        decode: impl Fn(&[u8]) -> anyhow::Result<GoveeBlePacket> + 'static + Sync + Send,
    ) -> Self {
        Self {
            encode: Box::new(move |any| {
                // This downcast cannot fail. PacketCodec::new is the only
                // constructor and welds the encoder's captured T to the
                // type_id field via a single type parameter, so T here is
                // the same T whose TypeId was stored at line 52.
                // PacketManager keys its table by TypeId and resolves with
                // TypeId::of::<T_caller>(), so a successful lookup proves
                // T_caller == T_encoder, and TypeIds are unique per type.
                let Some(value) = any.downcast_ref::<T>() else {
                    unreachable!(
                        "PacketCodec::new welds type_id to encoder T; \
                         downcast cannot fail if the registry lookup succeeded"
                    );
                };
                (encode)(value)
            }),
            decode: Box::new(decode),
            supported_skus,
            type_id: TypeId::of::<T>(),
        }
    }
}

pub struct PacketManager {
    codec_by_sku: Mutex<HashMap<String, HashMap<TypeId, Arc<PacketCodec>>>>,
    all_codecs: Vec<Arc<PacketCodec>>,
}

impl PacketManager {
    fn map_for_sku(&self, sku: &str) -> MappedMutexGuard<'_, HashMap<TypeId, Arc<PacketCodec>>> {
        MutexGuard::map(self.codec_by_sku.lock(), |codecs| {
            codecs.entry(sku.to_string()).or_insert_with(|| {
                let mut map = HashMap::new();

                for codec in &self.all_codecs {
                    if codec.supported_skus.contains(&sku)
                        && map.insert(codec.type_id, codec.clone()).is_some()
                    {
                        log::error!("Conflicting PacketCodecs for {sku} {:?}", codec.type_id);
                    }
                }

                map
            })
        })
    }

    fn resolve_by_sku(&self, sku: &str, type_id: &TypeId) -> ApiResult<Arc<PacketCodec>> {
        let map = self.map_for_sku(sku);

        map.get(type_id).cloned().ok_or_else(|| {
            GoveeApiError::Unsupported(format!("sku {sku} has no codec for type {type_id:?}"))
        })
    }

    pub fn decode_for_sku(&self, sku: &str, data: &[u8]) -> GoveeBlePacket {
        let map = self.map_for_sku(sku);

        for codec in map.values() {
            if let Ok(value) = (codec.decode)(data) {
                return value;
            }
        }

        GoveeBlePacket::Generic(HexBytes(data.to_vec()))
    }

    pub fn encode_for_sku<T: 'static>(&self, sku: &str, value: &T) -> ApiResult<Vec<u8>> {
        let type_id = TypeId::of::<T>();
        let codec = self.resolve_by_sku(sku, &type_id)?;

        (codec.encode)(value).map_err(classify_codec_error)
    }

    pub fn new() -> Self {
        let mut all_codecs = vec![];

        crate::ble::humidifier::register(&mut all_codecs);
        crate::ble::light::register(&mut all_codecs);
        crate::ble::projector::register(&mut all_codecs);

        Self {
            codec_by_sku: Mutex::new(HashMap::new()),
            all_codecs: all_codecs.into_iter().map(Arc::new).collect(),
        }
    }
}

/// Map a codec closure's anyhow error to a public variant. The two codec
/// closures that bail with `CodecUnsupported` (decoders for write-only
/// frames) become `GoveeApiError::Unsupported`; everything else from the
/// codec layer is a wire-format failure and becomes `Protocol`.
fn classify_codec_error(err: anyhow::Error) -> GoveeApiError {
    if let Some(unsupported) = err.downcast_ref::<CodecUnsupported>() {
        return GoveeApiError::Unsupported(unsupported.0.to_string());
    }
    GoveeApiError::Protocol(format!("{err:#}"))
}

impl Default for PacketManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Emit the bytes for one field or constant when encoding a packet body. Used by
/// the `packet!` macro; defined at module scope so the family modules can call it.
#[macro_export]
macro_rules! encode_body {
    // Tail case: nothing to do
    ($target:expr,$input:expr,) => {};

    // Match a constant byte; emit it
    ($target:expr,$input:expr, $expected:literal, $($tail:tt)*) => {
            $target.push($expected);
            $crate::encode_body!($target, $input, $($tail)*);
    };

    // Match a field; emit it from the struct
    ($target:expr, $input:expr, $field_name:ident, $($tail:tt)*) => {
            $crate::ble::codec::DecodePacketParam::encode_param(&$input.$field_name, $target);
            $crate::encode_body!($target, $input, $($tail)*);
    };
}

/// Parse one field or verify one constant byte when decoding a packet body. Used
/// by the `packet!` macro.
#[macro_export]
macro_rules! decode_body {
    // Tail case; verify that remaining bytes are zero
    ($target:expr, $data:expr,) => {
        while !$data.is_empty() {
            anyhow::ensure!($data[0] == 0);
            $data = &$data[1..];
        }
    };

    // Match a constant byte; check that it is what we expect
    ($target:expr, $data:expr, $expected:literal, $($tail:tt)*) => {
            let maybe_byte = $data.get(0);
            anyhow::ensure!(maybe_byte == Some(&$expected),"expected {} but got {maybe_byte:?}", $expected);
            $data = &$data[1..];
            $crate::decode_body!($target, $data, $($tail)*);
    };

    // Match a field; parse it into the struct
    ($target:expr, $data:expr, $field_name:ident, $($tail:tt)*) => {
            let remain = $crate::ble::codec::DecodePacketParam::decode_param(&mut $target.$field_name, $data)?;
            $data = remain;
            $crate::decode_body!($target, $data, $($tail)*);
    };
}

/// Helper for defining a PacketCodec.
/// The first param is the list of SKUs which are known to support
/// this packet.
/// The second parameter is the name of the type which will be
/// encoded into raw bytes when encoding. It must impl Default.
/// The third parameter is the name of the GoveeBlePacket enum
/// variant that holds that type.
/// The subsequent parameters are rules that match the bytes
/// in the packet when decoding, or form the bytes in the packet
/// when encoding. They are listed in the same sequence that they
/// have in the packet.
#[macro_export]
macro_rules! packet {
    ($skus:expr, $struct:ident, $variant:ident, $($body:tt)*) => {
        $crate::ble::codec::PacketCodec::new(
            $skus,
            |input_value: &$struct| {
                let mut bytes = vec![];
                $crate::encode_body!(&mut bytes, input_value, $($body)*);
                Ok($crate::ble::codec::finish(bytes))
            },
            |data| {
                let mut data = &data[0..data.len().saturating_sub(1)];
                let mut value = $struct::default();
                $crate::decode_body!(&mut value, data, $($body)*);
                Ok($crate::ble::codec::GoveeBlePacket::$variant(value))
            }
        )
    }
}

pub trait DecodePacketParam {
    fn decode_param<'a>(&mut self, data: &'a [u8]) -> anyhow::Result<&'a [u8]>;
    fn encode_param(&self, target: &mut Vec<u8>);
}

impl DecodePacketParam for u8 {
    fn decode_param<'a>(&mut self, data: &'a [u8]) -> anyhow::Result<&'a [u8]> {
        *self = *data.first().ok_or_else(|| anyhow!("EOF"))?;
        Ok(&data[1..])
    }

    fn encode_param(&self, target: &mut Vec<u8>) {
        target.push(*self);
    }
}

impl DecodePacketParam for bool {
    fn decode_param<'a>(&mut self, data: &'a [u8]) -> anyhow::Result<&'a [u8]> {
        let mut byte = 0u8;
        let remain = byte.decode_param(data)?;
        *self = byte != 0;
        Ok(remain)
    }

    fn encode_param(&self, target: &mut Vec<u8>) {
        target.push(u8::from(*self));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoveeBlePacket {
    Generic(HexBytes),
    #[allow(unused)] // can remove if/when SetSceneCode::decode has an impl
    SetSceneCode(super::light::SetSceneCode),
    SetDevicePower(super::light::SetDevicePower),
    SetHumidifierNightlight(super::humidifier::SetHumidifierNightlightParams),
    NotifyHumidifierMode(super::humidifier::NotifyHumidifierMode),
    SetHumidifierMode(super::humidifier::SetHumidifierMode),
    NotifyHumidifierAutoMode(super::humidifier::HumidifierAutoMode),
    NotifyHumidifierNightlight(super::humidifier::NotifyHumidifierNightlightParams),
    SetPairingStatus(super::projector::SetPairingStatus),
    SetPairingSound(super::projector::SetPairingSound),
    SetSilentPowerUp(super::projector::SetSilentPowerUp),
    SetDreamViewLaser(super::projector::SetDreamViewLaser),
    SetAutoOff(super::projector::SetAutoOff),
    SetAurora(super::projector::SetAurora),
    NotifyAurora(super::projector::NotifyAurora),
    NotifyLaser(super::projector::NotifyLaser),
}

#[derive(Debug)]
pub struct Base64HexBytes(pub(crate) HexBytes);

impl Base64HexBytes {
    pub fn decode_for_sku(&self, sku: &str) -> GoveeBlePacket {
        super::MGR.decode_for_sku(sku, &self.0.0)
    }

    pub fn encode_for_sku<T: 'static>(sku: &str, value: &T) -> ApiResult<Self> {
        super::MGR
            .encode_for_sku(sku, value)
            .map(|bytes| Base64HexBytes(HexBytes(bytes)))
    }

    pub fn base64(&self) -> Vec<String> {
        let mut result = vec![];
        for chunk in self.0.0.chunks(20) {
            result.push(data_encoding::BASE64.encode(chunk));
        }
        result
    }

    /// The raw frame bytes (20 bytes per command). Used by the BLE transport,
    /// which writes them to the device directly rather than base64-wrapping them
    /// into a cloud message.
    pub fn bytes(&self) -> &[u8] {
        &self.0.0
    }

    pub fn with_bytes(bytes: Vec<u8>) -> Self {
        Self(HexBytes(finish(bytes)))
    }
}

impl<'de> Deserialize<'de> for Base64HexBytes {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error as _;
        let encoded = String::deserialize(deserializer)?;
        let decoded = data_encoding::BASE64
            .decode(encoded.as_ref())
            .map_err(|e| D::Error::custom(format!("{e:#}")))?;
        Ok(Self(HexBytes(decoded)))
    }
}

fn calculate_checksum(data: &[u8]) -> u8 {
    let mut checksum: u8 = 0;
    for &b in data {
        checksum ^= b;
    }
    checksum
}

/// Pad a packet body out to 19 bytes and append the XOR checksum as byte 20.
pub fn finish(mut data: Vec<u8>) -> Vec<u8> {
    let checksum = calculate_checksum(&data);
    data.resize(19, 0);
    data.push(checksum);
    data
}
