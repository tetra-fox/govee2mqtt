//! BLE command structs for H7160-class humidifiers and the codecs that encode
//! and decode them. See hatest/api-map/04-sku-encoders.md for how these opcodes
//! line up with the app's H71xx controllers.

use super::codec::{DecodePacketParam, PacketCodec};
use crate::packet;
use serde::Serialize;

/// Register every humidifier codec into the PacketManager's table.
pub(super) fn register(codecs: &mut Vec<PacketCodec>) {
    codecs.push(packet!(
        &["H7160"],
        SetHumidifierMode,
        SetHumidifierMode,
        0x33,
        0x05,
        mode,
        param,
    ));
    codecs.push(packet!(
        &["H7160"],
        NotifyHumidifierMode,
        NotifyHumidifierMode,
        0xaa,
        0x05,
        0x00,
        mode,
        param,
    ));
    codecs.push(packet!(
        &["H7160"],
        HumidifierAutoMode,
        NotifyHumidifierAutoMode,
        0xaa,
        0x05,
        0x03,
        target_humidity,
    ));
    codecs.push(packet!(
        &["H7160"],
        NotifyHumidifierNightlightParams,
        NotifyHumidifierNightlight,
        0xaa,
        0x1b,
        on,
        brightness,
        r,
        g,
        b,
    ));
    codecs.push(packet!(
        &["H7160"],
        SetHumidifierNightlightParams,
        SetHumidifierNightlight,
        0x33,
        0x1b,
        on,
        brightness,
        r,
        g,
        b,
    ));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SetHumidifierNightlightParams {
    pub on: bool,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub brightness: u8,
}

impl From<NotifyHumidifierNightlightParams> for SetHumidifierNightlightParams {
    fn from(val: NotifyHumidifierNightlightParams) -> Self {
        SetHumidifierNightlightParams {
            on: val.on,
            r: val.r,
            g: val.g,
            b: val.b,
            brightness: val.brightness,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
pub struct NotifyHumidifierNightlightParams {
    pub on: bool,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub brightness: u8,
}

/// Data is offset by 128 with increments of 1%,
/// so 0% is 128, 100% is 228
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetHumidity(u8);

impl From<TargetHumidity> for u8 {
    fn from(val: TargetHumidity) -> Self {
        val.0
    }
}

impl DecodePacketParam for TargetHumidity {
    fn decode_param<'a>(&mut self, data: &'a [u8]) -> anyhow::Result<&'a [u8]> {
        self.0.decode_param(data)
    }

    fn encode_param(&self, target: &mut Vec<u8>) {
        target.push(self.0);
    }
}

impl TargetHumidity {
    pub fn as_percent(&self) -> u8 {
        self.0 & 0x7f
    }

    pub fn into_inner(self) -> u8 {
        self.0
    }

    pub fn from_percent(percent: u8) -> Self {
        Self(percent + 128)
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct SetHumidifierMode {
    pub mode: u8,
    pub param: u8,
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct NotifyHumidifierMode {
    pub mode: u8,
    pub param: u8,
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct HumidifierAutoMode {
    pub target_humidity: TargetHumidity,
}
