//! The device capability model shared across the Govee APIs.
//!
//! These types describe what a device can do and how its parameters are shaped.
//! The platform API returns them directly; the undocumented API synthesizes some
//! of them (eg: scene lists). The rest of the codebase pattern-matches on
//! [`DeviceCapabilityKind`] and [`DeviceParameters`] to decide how to control and
//! represent a device, so this is the type model that grows as new device types
//! and capabilities are added.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Helper to generate boilerplate around govee enum string types
macro_rules! enum_string {
    {pub enum $name:ident {
     $($var:ident = $label:literal),* $(,)?
     }
    } => {

#[derive(Debug, Clone, PartialEq, Eq, strum_macros::Display, strum_macros::EnumString)]
pub enum $name {
    $(
        #[strum(serialize = $label)]
        $var,
    )*
        Other(String),
}

impl Default for $name {
    fn default() -> Self {
        Self::Other("NONE".to_string())
    }
}

impl<'de> Deserialize<'de> for $name {
    fn deserialize<D>(d: D) -> Result<Self, <D as Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(d)?;

        if let Ok(t) = s.parse::<Self>() {
            Ok(t)
        } else {
            Ok(Self::Other(s))
        }
    }
}

impl Serialize for $name {
    fn serialize<S>(&self, serializer: S) -> Result<<S as Serializer>::Ok, <S as Serializer>::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Other(s) => s.serialize(serializer),
            _ => self.to_string().serialize(serializer),
        }
    }
}

    }
}

enum_string! {
pub enum DeviceType {
    Light = "devices.types.light",
    AirPurifier = "devices.types.air_purifier",
    Thermometer = "devices.types.thermometer",
    Socket = "devices.types.socket",
    Sensor = "devices.types.sensor",
    Heater = "devices.types.heater",
    Humidifier = "devices.types.humidifier",
    Dehumidifier = "devices.types.dehumidifier",
    IceMaker = "devices.types.ice_maker",
    AromaDiffuser = "devices.types.aroma_diffuser",
    Fan = "devices.types.fan",
    Kettle = "devices.types.kettle",
}
}

enum_string! {
pub enum DeviceCapabilityKind {
    OnOff = "devices.capabilities.on_off",
    Toggle = "devices.capabilities.toggle",
    Range = "devices.capabilities.range",
    Mode = "devices.capabilities.mode",
    ColorSetting = "devices.capabilities.color_setting",
    SegmentColorSetting = "devices.capabilities.segment_color_setting",
    MusicSetting = "devices.capabilities.music_setting",
    DynamicScene = "devices.capabilities.dynamic_scene",
    WorkMode = "devices.capabilities.work_mode",
    DynamicSetting = "devices.capabilities.dynamic_setting",
    TemperatureSetting = "devices.capabilities.temperature_setting",
    Online = "devices.capabilities.online",
    Property = "devices.capabilities.property",
    Event = "devices.capabilities.event",
}
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct DeviceCapability {
    #[serde(rename = "type")]
    pub kind: DeviceCapabilityKind,
    pub instance: String,
    pub parameters: Option<DeviceParameters>,
    #[serde(rename = "alarmType")]
    pub alarm_type: Option<u32>,
    #[serde(rename = "eventState")]
    pub event_state: Option<JsonValue>,
}

impl DeviceCapability {
    pub fn enum_parameter_by_name(&self, name: &str) -> Option<u32> {
        self.parameters
            .as_ref()
            .and_then(|p| p.enum_parameter_by_name(name))
    }

    pub fn struct_field_by_name(&self, name: &str) -> Option<&StructField> {
        match &self.parameters {
            Some(DeviceParameters::Struct { fields }) => {
                fields.iter().find(|f| f.field_name == name)
            }
            _ => None,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(tag = "dataType")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub enum DeviceParameters {
    #[serde(rename = "ENUM")]
    Enum { options: Vec<EnumOption> },
    #[serde(rename = "INTEGER")]
    Integer {
        unit: Option<String>,
        range: IntegerRange,
    },
    #[serde(rename = "STRUCT")]
    Struct { fields: Vec<StructField> },
    #[serde(rename = "Array")]
    Array {
        size: Option<ArraySize>,
        #[serde(rename = "elementRange")]
        element_range: Option<ElementRange>,
        #[serde(rename = "elementType")]
        element_type: Option<String>,
        #[serde(default)]
        options: Vec<ArrayOption>,
    },
}

impl DeviceParameters {
    pub fn enum_parameter_by_name(&self, name: &str) -> Option<u32> {
        match self {
            DeviceParameters::Enum { options } => options
                .iter()
                .find(|e| e.name == name && e.value.is_i64())
                .map(|e| e.value.as_i64().expect("i64") as u32),
            _ => None,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
// No deny_unknown_fields here, because we embed via flatten
pub struct StructField {
    #[serde(rename = "fieldName")]
    pub field_name: String,

    #[serde(flatten)]
    pub field_type: DeviceParameters,

    #[serde(rename = "defaultValue")]
    pub default_value: Option<JsonValue>,

    #[serde(default)]
    pub required: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct ElementRange {
    pub min: u32,
    pub max: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct ArraySize {
    pub min: u32,
    pub max: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct IntegerRange {
    pub min: u32,
    pub max: u32,
    pub precision: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct EnumOption {
    pub name: String,
    #[serde(default)]
    pub value: JsonValue,
    #[serde(flatten)]
    pub extras: HashMap<String, JsonValue>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct ArrayOption {
    pub value: u32,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn enum_repr() {
        assert_eq!(
            serde_json::to_string(&DeviceType::Light).unwrap(),
            "\"devices.types.light\""
        );
        assert_eq!(
            serde_json::to_string(&DeviceType::Other("something".to_string())).unwrap(),
            "\"something\""
        );
    }
}
