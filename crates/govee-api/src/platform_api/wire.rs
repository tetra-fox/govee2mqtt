use crate::model::*;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Deserialize, Serialize, Debug)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub(super) struct GetDeviceScenesResponse {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub code: u32,
    #[serde(rename = "msg")]
    pub message: String,
    pub payload: GetDeviceScenesResponsePayload,
}

#[derive(Deserialize, Serialize, Debug)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub(super) struct GetDeviceScenesResponsePayload {
    pub sku: String,
    pub device: String,
    pub capabilities: Vec<DeviceCapability>,
}

#[derive(Serialize, Debug)]
pub(super) struct GetDeviceScenesRequest {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub payload: GetDeviceScenesPayload,
}

#[derive(Serialize, Debug)]
pub(super) struct GetDeviceScenesPayload {
    pub sku: String,
    pub device: String,
}

#[derive(Serialize, Debug)]
pub(super) struct ControlDeviceRequest {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub payload: ControlDevicePayload,
}

#[derive(Serialize, Debug)]
pub(super) struct ControlDevicePayload {
    pub sku: String,
    pub device: String,
    pub capability: ControlDeviceCapability,
}

#[derive(Serialize, Debug)]
pub(super) struct ControlDeviceCapability {
    #[serde(rename = "type")]
    pub kind: DeviceCapabilityKind,
    pub instance: String,
    pub value: JsonValue,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub(super) struct ControlDeviceResponse {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub code: u32,
    #[serde(rename = "msg")]
    pub message: String,

    pub capability: ControlDeviceResponseCapability,
}

#[derive(Deserialize, Debug)]
#[allow(unused)]
pub struct ControlDeviceResponseCapability {
    #[serde(rename = "type")]
    pub kind: DeviceCapabilityKind,
    pub instance: String,
    pub value: JsonValue,
    pub state: JsonValue,
}

#[derive(Serialize, Debug)]
pub(super) struct GetDeviceStateRequest {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub payload: GetDeviceStateRequestPayload,
}

#[derive(Serialize, Debug)]
pub(super) struct GetDeviceStateRequestPayload {
    pub sku: String,
    pub device: String,
}

#[derive(Deserialize, Serialize, Debug)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub(super) struct GetDeviceStateResponse {
    #[serde(rename = "requestId")]
    pub request_id: String,
    pub code: u32,
    #[serde(rename = "msg")]
    pub message: String,
    pub payload: HttpDeviceState,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct HttpDeviceState {
    pub sku: String,
    pub device: String,
    pub capabilities: Vec<DeviceCapabilityState>,
}

impl HttpDeviceState {
    pub fn capability_by_instance(&self, instance: &str) -> Option<&DeviceCapabilityState> {
        self.capabilities
            .iter()
            .find(|c| c.instance.eq_ignore_ascii_case(instance))
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(tag = "type")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct DeviceCapabilityState {
    #[serde(rename = "type")]
    pub kind: DeviceCapabilityKind,
    pub instance: String,
    pub state: JsonValue,
}

#[derive(Deserialize, Serialize, Debug)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub(super) struct GetDevicesResponse {
    pub code: u32,
    pub message: String,
    pub data: Vec<HttpDeviceInfo>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct HttpDeviceInfo {
    pub sku: String,
    pub device: String,
    #[serde(default, rename = "deviceName")]
    pub device_name: String,
    #[serde(default, rename = "type")]
    pub device_type: DeviceType,
    pub capabilities: Vec<DeviceCapability>,
}

impl HttpDeviceInfo {
    pub fn capability_by_instance(&self, instance: &str) -> Option<&DeviceCapability> {
        self.capabilities
            .iter()
            .find(|c| c.instance.eq_ignore_ascii_case(instance))
    }

    pub fn supports_rgb(&self) -> bool {
        self.capability_by_instance("colorRgb").is_some()
    }

    pub fn supports_brightness(&self) -> bool {
        self.capability_by_instance("brightness").is_some()
    }

    pub fn supports_dynamic_scenes(&self) -> bool {
        self.capabilities
            .iter()
            .any(|cap| cap.kind == DeviceCapabilityKind::DynamicScene)
    }

    /// If supported, returns the number of segments
    pub fn supports_segmented_rgb(&self) -> Option<std::ops::Range<u32>> {
        let cap = self.capability_by_instance("segmentedColorRgb")?;
        let field = cap.struct_field_by_name("segment")?;
        match field.field_type {
            DeviceParameters::Array {
                size:
                    Some(ArraySize {
                        // These are the display indices. eg: 1-based
                        min: label_min,
                        max: label_max,
                    }),
                element_range:
                    Some(ElementRange {
                        // These are the actual indices. eg: 0-based
                        min: range_min,
                        // We ignore the max here, because the data
                        // reported by Govee can be bogus:
                        // <https://developer.govee.com/discuss/6599afb91cb48d002dbed2b8>
                        max: _,
                    }),
                ..
            } => {
                // This range is an inclusive range, so add 1
                let num_segments = (1 + label_max).saturating_sub(label_min);
                // Return our exclusive range
                Some(range_min..range_min + num_segments)
            }
            _ => None,
        }
    }

    pub fn supports_segmented_brightness(&self) -> Option<(u32, u32)> {
        let cap = self.capability_by_instance("segmentedBrightness")?;
        let field = cap.struct_field_by_name("brightness")?;
        match &field.field_type {
            DeviceParameters::Integer {
                range: IntegerRange { min, max, .. },
                ..
            } => Some((*min, *max)),
            _ => None,
        }
    }

    pub fn get_color_temperature_range(&self) -> Option<(u32, u32)> {
        let cap = self.capability_by_instance("colorTemperatureK")?;

        match cap.parameters {
            Some(DeviceParameters::Integer {
                range: IntegerRange { min, max, .. },
                ..
            }) => Some((min, max)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::http::from_json;

    const SCENE_LIST: &str = include_str!("../../test-data/scenes.json");

    #[test]
    fn get_device_scenes() {
        let _: GetDeviceScenesResponse = from_json(SCENE_LIST).expect("parse device scenes");
    }

    const GET_DEVICE_STATE_EXAMPLE: &str = include_str!("../../test-data/get_device_state.json");

    #[test]
    fn get_device_state() {
        let _: GetDeviceStateResponse =
            from_json(GET_DEVICE_STATE_EXAMPLE).expect("parse device state");
    }

    const LIST_DEVICES_EXAMPLE: &str = include_str!("../../test-data/list_devices.json");
    const LIST_DEVICES_EXAMPLE2: &str = include_str!("../../test-data/list_devices_2.json");

    #[test]
    fn list_devices_issue4() {
        let resp: GetDevicesResponse =
            from_json(include_str!("../../test-data/list_devices_issue4.json")).expect("parse");
        assert!(!resp.data.is_empty());
    }

    #[test]
    fn list_devices_2() {
        let resp: GetDevicesResponse = from_json(LIST_DEVICES_EXAMPLE2).expect("parse");
        assert!(!resp.data.is_empty());
    }

    #[test]
    fn list_devices() {
        let resp: GetDevicesResponse = from_json(LIST_DEVICES_EXAMPLE).expect("parse");
        assert!(!resp.data.is_empty());
    }
}
