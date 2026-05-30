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
use serde::{Deserialize, Deserializer, Serialize};
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

/// Sentinel `supported_skus` entry for codecs that apply to every device, not a
/// single family: the protocol-level frames in [`super::common`] (the keepalive
/// and power-state notify). A codec listing this is folded into every SKU's map.
pub const ALL_SKUS: &str = "*";

#[derive(Clone, PartialEq, Eq)]
pub struct HexBytes(pub(crate) Vec<u8>);

impl std::fmt::Debug for HexBytes {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.write_fmt(format_args!("{:02X?}", self.0))
    }
}

/// One byte of a packet body, as the `packet!` macro lays it out: either a
/// fixed constant or a named struct field. The annotation layer walks this to
/// map byte offsets back to the names the codec already uses, so the field
/// names in a `packet!` definition double as the inspector's per-byte docs.
#[derive(Clone, Copy, Debug)]
pub enum FieldSpec {
    Const(u8),
    Field(&'static str),
}

#[allow(clippy::type_complexity)]
pub struct PacketCodec {
    encode: Box<dyn Fn(&dyn Any) -> anyhow::Result<Vec<u8>> + Sync + Send>,
    decode: Box<dyn Fn(&[u8]) -> anyhow::Result<GoveeBlePacket> + Sync + Send>,
    supported_skus: &'static [&'static str],
    type_id: TypeId,
    /// One entry per body byte, in order, or empty for hand-written codecs that
    /// don't declare a layout (the annotation falls back to structural-only).
    field_specs: Vec<FieldSpec>,
}

impl PacketCodec {
    pub fn new<T: 'static>(
        supported_skus: &'static [&'static str],
        encode: impl Fn(&T) -> anyhow::Result<Vec<u8>> + 'static + Sync + Send,
        decode: impl Fn(&[u8]) -> anyhow::Result<GoveeBlePacket> + 'static + Sync + Send,
    ) -> Self {
        Self {
            field_specs: Vec::new(),
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

    /// Attach the per-byte layout for the annotation layer. Called by the
    /// `packet!` macro with the specs derived from the packet body; hand-written
    /// codecs may call it to describe their layout too.
    pub fn with_field_specs(mut self, specs: Vec<FieldSpec>) -> Self {
        self.field_specs = specs;
        self
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
                    if (codec.supported_skus.contains(&sku)
                        || codec.supported_skus.contains(&ALL_SKUS))
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

    /// Annotate a raw frame for the inspector: a one-line summary plus a per-byte
    /// field map. When a codec decodes the frame, its declared layout names the
    /// data bytes; otherwise only the structural bytes (family, checksum, zero
    /// padding) are labelled.
    pub fn annotate_for_sku(&self, sku: &str, data: &[u8]) -> FrameAnnotation {
        let map = self.map_for_sku(sku);
        for codec in map.values() {
            if let Ok(value) = (codec.decode)(data) {
                return FrameAnnotation {
                    summary: value.label().to_string(),
                    fields: annotate_fields(data, &codec.field_specs),
                };
            }
        }
        FrameAnnotation {
            summary: undecoded_summary(data),
            fields: annotate_fields(data, &[]),
        }
    }

    pub fn encode_for_sku<T: 'static>(&self, sku: &str, value: &T) -> ApiResult<Vec<u8>> {
        let type_id = TypeId::of::<T>();
        let codec = self.resolve_by_sku(sku, &type_id)?;

        (codec.encode)(value).map_err(classify_codec_error)
    }

    pub fn new() -> Self {
        let mut all_codecs = vec![];

        crate::ble::common::register(&mut all_codecs);
        crate::ble::humidifier::register(&mut all_codecs);
        crate::ble::light::register(&mut all_codecs);
        crate::ble::projector::register(&mut all_codecs);
        crate::ble::socket::register(&mut all_codecs);

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

/// Record one [`FieldSpec`](crate::ble::codec::FieldSpec) per body byte, in
/// order, so the annotation layer can map offsets back to field names. Mirrors
/// the structure of `decode_body!`. Used by the `packet!` macro.
#[macro_export]
macro_rules! describe_body {
    ($specs:expr,) => {};

    // Constant byte
    ($specs:expr, $expected:literal, $($tail:tt)*) => {
            $specs.push($crate::ble::codec::FieldSpec::Const($expected));
            $crate::describe_body!($specs, $($tail)*);
    };

    // Named field
    ($specs:expr, $field_name:ident, $($tail:tt)*) => {
            $specs.push($crate::ble::codec::FieldSpec::Field(stringify!($field_name)));
            $crate::describe_body!($specs, $($tail)*);
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
        .with_field_specs({
            let mut specs = vec![];
            $crate::describe_body!(&mut specs, $($body)*);
            specs
        })
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
    SetBrightness(super::light::SetBrightness),
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
    SetCountdown(super::socket::SetCountdown),
    NotifyCountdown(super::socket::NotifyCountdown),
    SetTimerSlot(super::socket::SetTimerSlot),
    NotifyTimerCount(super::socket::NotifyTimerCount),
    NotifyPower(super::common::NotifyPower),
    NotifyKeepalive(super::common::NotifyKeepalive),
}

impl GoveeBlePacket {
    /// One-line human summary for the inspector, naming the operation. This is
    /// where an opcode's prose name lives, alongside the variant the codec
    /// decodes into, so the inspector documents the protocol from one place.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Generic(_) => "unknown",
            Self::SetSceneCode(_) => "set scene code",
            Self::SetDevicePower(_) => "set power",
            Self::SetBrightness(_) => "set brightness",
            Self::SetHumidifierNightlight(_) => "set humidifier nightlight",
            Self::NotifyHumidifierMode(_) => "humidifier mode notify",
            Self::SetHumidifierMode(_) => "set humidifier mode",
            Self::NotifyHumidifierAutoMode(_) => "humidifier auto-mode notify",
            Self::NotifyHumidifierNightlight(_) => "humidifier nightlight notify",
            Self::SetPairingStatus(_) => "set pairing status",
            Self::SetPairingSound(_) => "set pairing sound",
            Self::SetSilentPowerUp(_) => "set silent power-up",
            Self::SetDreamViewLaser(_) => "set DreamView laser",
            Self::SetAutoOff(_) => "set auto-off",
            Self::SetAurora(_) => "set aurora",
            Self::NotifyAurora(_) => "aurora notify",
            Self::NotifyLaser(_) => "laser notify",
            Self::SetCountdown(_) => "set countdown",
            Self::NotifyCountdown(_) => "countdown notify",
            Self::SetTimerSlot(_) => "set timer slot",
            Self::NotifyTimerCount(_) => "timer count notify",
            Self::NotifyPower(_) => "power state notify",
            Self::NotifyKeepalive(_) => "keepalive",
        }
    }
}

/// Per-byte role in a 20-byte frame, for the inspector. `Family` is byte 0,
/// `Opcode` byte 1, `Field`/`Const` are the data bytes a codec declared,
/// `Padding` is the zero fill, `Checksum` is the trailing XOR.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FieldRole {
    Family,
    Opcode,
    Field,
    Const,
    Padding,
    Checksum,
    Unknown,
}

/// One labelled byte (or span) of a frame.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct FieldNote {
    pub offset: usize,
    pub len: usize,
    pub role: FieldRole,
    pub label: String,
}

/// A decoded frame's inspector annotation: a summary plus the per-byte map.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct FrameAnnotation {
    pub summary: String,
    pub fields: Vec<FieldNote>,
}

/// Human family name for byte 0, by the high-level frame type it marks.
fn family_label(byte: u8) -> &'static str {
    match byte {
        0x33 => "write",
        0xaa => "read/notify",
        0xa3 => "live blob",
        0xe7 => "handshake",
        0xee => "device notify",
        _ => "family",
    }
}

/// Summary for a frame no codec decoded: name the family so the inspector still
/// says something useful (and flags that the bytes might be pre-decryption
/// ciphertext, which has no recognizable family byte).
fn undecoded_summary(data: &[u8]) -> String {
    match data.first() {
        Some(&b) if family_label(b) != "family" => format!("undecoded {} frame", family_label(b)),
        Some(_) => "undecoded (or pre-decryption ciphertext)".to_string(),
        None => "(empty)".to_string(),
    }
}

/// Build the per-byte field map. `specs` names the data bytes in order (empty
/// for undecoded frames); bytes 0 and 1 are always family and opcode, byte 19 is
/// the XOR checksum.
///
/// For a byte the codec didn't name, we only call it padding when it is in the
/// trailing run of zeros (the Govee framing zero-pads to 19 bytes). An interior
/// zero, i.e. one before the last non-zero data byte, is a field value of 0, not
/// padding, so we mark it unknown rather than guess it away.
fn annotate_fields(data: &[u8], specs: &[FieldSpec]) -> Vec<FieldNote> {
    let mut notes = Vec::with_capacity(data.len());
    let full = data.len() == 20;
    let body_end = if full { 19 } else { data.len() };
    // Highest data offset that carries a non-zero value; zeros past it are the
    // trailing pad. None means the whole body is zero.
    let last_nonzero = (2..body_end).rev().find(|&i| data[i] != 0);
    for (offset, &value) in data.iter().enumerate() {
        let (role, label) = if offset == 0 {
            (FieldRole::Family, family_label(value).to_string())
        } else if offset == 1 {
            (FieldRole::Opcode, format!("opcode {value:#04x}"))
        } else if full && offset == 19 {
            (FieldRole::Checksum, "xor checksum".to_string())
        } else if offset < body_end {
            // data bytes: the first two specs are the family/opcode handled
            // above, so the data byte at `offset` is spec index `offset`.
            match specs.get(offset) {
                Some(FieldSpec::Field(name)) => (FieldRole::Field, name.to_string()),
                Some(FieldSpec::Const(_)) => (FieldRole::Const, "const".to_string()),
                None if last_nonzero.is_some_and(|last| offset <= last) => {
                    (FieldRole::Unknown, String::new())
                }
                None => (FieldRole::Padding, "padding".to_string()),
            }
        } else {
            (FieldRole::Padding, "padding".to_string())
        };
        notes.push(FieldNote {
            offset,
            len: 1,
            role,
            label,
        });
    }
    notes
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
