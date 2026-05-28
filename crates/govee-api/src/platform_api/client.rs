use super::wire::{
    ControlDeviceCapability, ControlDevicePayload, ControlDeviceRequest, ControlDeviceResponse,
    ControlDeviceResponseCapability, GetDeviceScenesPayload, GetDeviceScenesRequest,
    GetDeviceScenesResponse, GetDeviceStateRequest, GetDeviceStateRequestPayload,
    GetDeviceStateResponse, GetDevicesResponse, HttpDeviceInfo, HttpDeviceState,
};
use super::{
    FIVE_MINUTES, GoveeApiClient, ONE_WEEK, endpoint, new_request_id,
    parse_temperature_constraints, sort_and_dedup_scenes,
};
use crate::cache::{CacheComputeResult, CacheGetOptions, cache_get};
use crate::error::{ApiResult, GoveeApiError};
use crate::model::*;
use crate::temperature::{TemperatureUnits, TemperatureValue};
use crate::undoc_api::GoveeUndocumentedApi;
use reqwest::Method;
use serde_json::{Value as JsonValue, json};
use std::time::Duration;

fn unsupported(message: impl Into<String>) -> GoveeApiError {
    GoveeApiError::Unsupported(message.into())
}

fn protocol(message: impl Into<String>) -> GoveeApiError {
    GoveeApiError::Protocol(message.into())
}

impl GoveeApiClient {
    pub async fn get_devices(&self) -> ApiResult<Vec<HttpDeviceInfo>> {
        cache_get(
            CacheGetOptions {
                topic: "http-api",
                key: "device-list",
                soft_ttl: Duration::from_secs(900),
                hard_ttl: ONE_WEEK,
                negative_ttl: Duration::from_secs(60),
                allow_stale: true,
            },
            async {
                let url = endpoint("/router/api/v1/user/devices");
                let resp: GetDevicesResponse = self.get_request_with_json_response(url).await?;
                Ok(CacheComputeResult::Value(resp.data))
            },
        )
        .await
    }

    pub async fn get_device_by_id(&self, id: &str) -> ApiResult<HttpDeviceInfo> {
        let devices = self.get_devices().await?;
        for d in devices {
            if d.device == id {
                return Ok(d);
            }
        }
        Err(unsupported(format!("device {id} not found")))
    }

    pub async fn control_device<V: Into<JsonValue>>(
        &self,
        device: &HttpDeviceInfo,
        capability: &DeviceCapability,
        value: V,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        let url = endpoint("/router/api/v1/device/control");
        let request = ControlDeviceRequest {
            request_id: new_request_id(),
            payload: ControlDevicePayload {
                sku: device.sku.to_string(),
                device: device.device.to_string(),
                capability: ControlDeviceCapability {
                    kind: capability.kind.clone(),
                    instance: capability.instance.to_string(),
                    value: value.into(),
                },
            },
        };

        let resp: ControlDeviceResponse = self
            .request_with_json_response(Method::POST, url, &request)
            .await?;

        log::trace!("control_device result: {resp:?}");

        Ok(resp.capability)
    }

    pub async fn get_device_state(&self, device: &HttpDeviceInfo) -> ApiResult<HttpDeviceState> {
        let url = endpoint("/router/api/v1/device/state");
        let request = GetDeviceStateRequest {
            request_id: new_request_id(),
            payload: GetDeviceStateRequestPayload {
                sku: device.sku.to_string(),
                device: device.device.to_string(),
            },
        };

        let resp: GetDeviceStateResponse = self
            .request_with_json_response(Method::POST, url, &request)
            .await?;

        Ok(resp.payload)
    }

    pub async fn get_device_diy_scenes(
        &self,
        device: &HttpDeviceInfo,
    ) -> ApiResult<Vec<DeviceCapability>> {
        if !device.supports_dynamic_scenes() {
            return Ok(vec![]);
        }

        let key = format!("scene-list-diy-{}-{}", device.sku, device.device);
        cache_get(
            CacheGetOptions {
                topic: "http-api",
                key: &key,
                soft_ttl: Duration::from_secs(300),
                hard_ttl: ONE_WEEK,
                negative_ttl: FIVE_MINUTES,
                allow_stale: true,
            },
            async {
                let url = endpoint("/router/api/v1/device/diy-scenes");
                let request = GetDeviceScenesRequest {
                    request_id: new_request_id(),
                    payload: GetDeviceScenesPayload {
                        sku: device.sku.to_string(),
                        device: device.device.to_string(),
                    },
                };

                let resp: GetDeviceScenesResponse = self
                    .request_with_json_response(Method::POST, url, &request)
                    .await?;

                Ok(CacheComputeResult::Value(resp.payload.capabilities))
            },
        )
        .await
    }

    pub async fn get_device_scenes(
        &self,
        device: &HttpDeviceInfo,
    ) -> ApiResult<Vec<DeviceCapability>> {
        if !device.supports_dynamic_scenes() {
            return Ok(vec![]);
        }

        let key = format!("scene-list-{}-{}", device.sku, device.device);
        cache_get(
            CacheGetOptions {
                topic: "http-api",
                key: &key,
                soft_ttl: Duration::from_secs(300),
                hard_ttl: ONE_WEEK,
                negative_ttl: FIVE_MINUTES,
                allow_stale: true,
            },
            async {
                let url = endpoint("/router/api/v1/device/scenes");
                let request = GetDeviceScenesRequest {
                    request_id: new_request_id(),
                    payload: GetDeviceScenesPayload {
                        sku: device.sku.to_string(),
                        device: device.device.to_string(),
                    },
                };

                let resp: GetDeviceScenesResponse = self
                    .request_with_json_response(Method::POST, url, &request)
                    .await?;

                Ok(CacheComputeResult::Value(resp.payload.capabilities))
            },
        )
        .await
    }

    pub async fn get_scene_caps(
        &self,
        device: &HttpDeviceInfo,
    ) -> ApiResult<Vec<DeviceCapability>> {
        let mut result = vec![];

        // These three fetches are independent; run them concurrently.
        let (scene_caps, diy_caps, undoc_caps) = tokio::join!(
            self.get_device_scenes(device),
            self.get_device_diy_scenes(device),
            GoveeUndocumentedApi::synthesize_platform_api_scene_list(&device.sku),
        );
        let scene_caps = scene_caps?;
        let diy_caps = diy_caps?;
        let undoc_caps = match undoc_caps {
            Ok(caps) => caps,
            Err(err) => {
                log::warn!("Synthesizing undoc scene list failed for {sku}: {err}", sku = device.sku);
                vec![]
            }
        };

        for (origin, caps) in [
            ("device.capabilities", &device.capabilities),
            ("scene_caps", &scene_caps),
            ("diy_caps", &diy_caps),
            ("undoc_caps", &undoc_caps),
        ] {
            for cap in caps {
                let is_scene = matches!(
                    cap.kind,
                    DeviceCapabilityKind::DynamicScene
                        | DeviceCapabilityKind::DynamicSetting
                        | DeviceCapabilityKind::Mode
                );
                if !is_scene {
                    continue;
                }

                match &cap.parameters {
                    Some(DeviceParameters::Enum { .. }) => {
                        result.push(cap.clone());
                    }
                    None => {
                        // This device has no scenes, skip it.
                    }
                    _ => {
                        log::warn!(
                            "Unexpected scene capability shape from {origin} for \
                             sku={sku} device={id}, ignoring: {cap:#?}",
                            sku = device.sku,
                            id = device.device
                        );
                    }
                }
            }
        }

        Ok(result)
    }

    pub async fn list_scene_names(&self, device: &HttpDeviceInfo) -> ApiResult<Vec<String>> {
        let mut result = vec![];

        let caps = self.get_scene_caps(device).await?;
        for cap in caps {
            match &cap.parameters {
                Some(DeviceParameters::Enum { options }) => {
                    for opt in options {
                        result.push(opt.name.to_string());
                    }
                }
                _ => {
                    return Err(protocol(format!(
                        "list_scene_names: unexpected scene capability shape {cap:#?}"
                    )));
                }
            }
        }

        // Add in music modes
        if let Some(cap) = device.capability_by_instance("musicMode")
            && let Some(DeviceParameters::Struct { fields }) = &cap.parameters
        {
            for f in fields {
                if f.field_name == "musicMode"
                    && let DeviceParameters::Enum { options } = &f.field_type
                {
                    for opt in options {
                        result.push(format!("Music: {}", opt.name));
                    }
                }
            }
        }

        if !result.is_empty() {
            result.insert(0, "".to_string());
        }

        Ok(sort_and_dedup_scenes(result))
    }

    pub async fn set_scene_by_name(
        &self,
        device: &HttpDeviceInfo,
        scene: &str,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        self.set_scene_by_name_with_music(device, scene, 100, true)
            .await
    }

    /// Like set_scene_by_name, but for the "Music: X" scenes lets the caller
    /// choose the sensitivity (0-100) and auto-color values that get sent with
    /// the music struct. For non-music scenes those arguments are ignored.
    pub async fn set_scene_by_name_with_music(
        &self,
        device: &HttpDeviceInfo,
        scene: &str,
        sensitivity: u8,
        auto_color: bool,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        if scene.is_empty() {
            return Err(unsupported("cannot set scene to the empty no-scene value"));
        }

        if let Some(music_mode) = scene.strip_prefix("Music: ")
            && device.capability_by_instance("musicMode").is_some()
        {
            return self
                .set_music_mode(device, music_mode, sensitivity, auto_color, None)
                .await;
        }

        let caps = self.get_scene_caps(device).await?;
        for cap in caps {
            match &cap.parameters {
                Some(DeviceParameters::Enum { options }) => {
                    for opt in options {
                        if scene.eq_ignore_ascii_case(&opt.name) {
                            return self.control_device(device, &cap, opt.value.clone()).await;
                        }
                    }
                }
                _ => {
                    return Err(protocol(format!(
                        "set_scene_by_name: unexpected scene capability shape {cap:#?}"
                    )));
                }
            }
        }
        Err(unsupported(format!(
            "scene '{scene}' is not available for this device"
        )))
    }

    /// Activate one of the device's music modes. The platform API musicMode
    /// struct carries musicMode (the mode value), sensitivity (0-100), autoColor
    /// (0/1) and rgb. rgb only has an effect when autoColor is off; it is sent
    /// only when a color is supplied, because the platform API rejects an
    /// explicit null rgb with "Parameter value cannot be empty".
    pub async fn set_music_mode(
        &self,
        device: &HttpDeviceInfo,
        mode: &str,
        sensitivity: u8,
        auto_color: bool,
        rgb: Option<u32>,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        let cap = device
            .capability_by_instance("musicMode")
            .ok_or_else(|| unsupported("device has no musicMode capability"))?;

        let mode_value = match cap.struct_field_by_name("musicMode").map(|f| &f.field_type) {
            Some(DeviceParameters::Enum { options }) => options
                .iter()
                .find(|opt| opt.name.eq_ignore_ascii_case(mode))
                .map(|opt| opt.value.clone())
                .ok_or_else(|| {
                    unsupported(format!(
                        "music mode '{mode}' is not available for this device"
                    ))
                })?,
            _ => {
                return Err(protocol(
                    "device musicMode capability has no musicMode enum field",
                ));
            }
        };

        let value = music_mode_value(mode_value, sensitivity, auto_color, rgb);
        self.control_device(device, cap, value).await
    }

    pub async fn set_target_temperature(
        &self,
        device: &HttpDeviceInfo,
        instance_name: &str,
        target: TemperatureValue,
        auto_stop: Option<bool>,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        let cap = device
            .capability_by_instance(instance_name)
            .ok_or_else(|| unsupported(format!("device has no {instance_name} capability")))?;

        let constraints = parse_temperature_constraints(cap)?.as_unit(TemperatureUnits::Celsius);

        let min = constraints.min.as_celsius();
        let max = constraints.max.as_celsius();
        let requested = target.as_celsius();
        let celsius = requested.max(min).min(max);
        if celsius != requested {
            log::info!(
                "Clamping target temperature {requested} to {celsius} (allowed range {min}..={max})"
            );
        }

        let mut value = json!({
            "temperature": celsius,
            "unit": "Celsius",
        });
        if let Some(auto_stop) = auto_stop {
            value["autoStop"] = json!(if auto_stop { 1 } else { 0 });
        }

        self.control_device(device, cap, value).await
    }

    pub async fn set_work_mode(
        &self,
        device: &HttpDeviceInfo,
        work_mode: i64,
        value: i64,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        let cap = device
            .capability_by_instance("workMode")
            .ok_or_else(|| unsupported("device has no workMode capability"))?;

        let value = json!({
            "workMode": work_mode,
            "modeValue": value
        });

        self.control_device(device, cap, value).await
    }

    pub async fn set_toggle_state(
        &self,
        device: &HttpDeviceInfo,
        instance: &str,
        on: bool,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        let cap = device
            .capability_by_instance(instance)
            .ok_or_else(|| unsupported(format!("device has no {instance} capability")))?;

        let value = cap
            .enum_parameter_by_name(if on { "on" } else { "off" })
            .ok_or_else(|| protocol(format!("{instance} capability has no on/off enum value")))?;

        self.control_device(device, cap, value).await
    }

    pub async fn set_power_state(
        &self,
        device: &HttpDeviceInfo,
        on: bool,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        self.set_toggle_state(device, "powerSwitch", on).await
    }

    pub async fn set_brightness(
        &self,
        device: &HttpDeviceInfo,
        percent: u8,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        let cap = device
            .capability_by_instance("brightness")
            .ok_or_else(|| unsupported("device has no brightness capability"))?;
        let value = match &cap.parameters {
            Some(DeviceParameters::Integer {
                range: IntegerRange { min, max, .. },
                ..
            }) => (percent as u32).max(*min).min(*max),
            _ => return Err(protocol("unexpected parameter type for brightness")),
        };
        self.control_device(device, cap, value).await
    }

    pub async fn set_color_temperature(
        &self,
        device: &HttpDeviceInfo,
        kelvin: u32,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        let cap = device
            .capability_by_instance("colorTemperatureK")
            .ok_or_else(|| unsupported("device has no colorTemperatureK capability"))?;
        let value = match &cap.parameters {
            Some(DeviceParameters::Integer {
                range: IntegerRange { min, max, .. },
                ..
            }) => (kelvin).max(*min).min(*max),
            _ => return Err(protocol("unexpected parameter type for colorTemperatureK")),
        };
        self.control_device(device, cap, value).await
    }

    pub async fn set_color_rgb(
        &self,
        device: &HttpDeviceInfo,
        r: u8,
        g: u8,
        b: u8,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        let cap = device
            .capability_by_instance("colorRgb")
            .ok_or_else(|| unsupported("device has no colorRgb capability"))?;
        let value = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
        self.control_device(device, cap, value).await
    }

    pub async fn set_segment_rgb(
        &self,
        device: &HttpDeviceInfo,
        segment: u32,
        r: u8,
        g: u8,
        b: u8,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        let cap = device
            .capability_by_instance("segmentedColorRgb")
            .ok_or_else(|| unsupported("device has no segmentedColorRgb capability"))?;
        let value = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
        self.control_device(
            device,
            cap,
            json!({
                "segment": vec![segment],
                "rgb": value,
            }),
        )
        .await
    }

    pub async fn set_segment_brightness(
        &self,
        device: &HttpDeviceInfo,
        segment: u32,
        percent: u8,
    ) -> ApiResult<ControlDeviceResponseCapability> {
        let cap = device
            .capability_by_instance("segmentedBrightness")
            .ok_or_else(|| unsupported("device has no segmentedBrightness capability"))?;

        let (min, max) = device
            .supports_segmented_brightness()
            .ok_or_else(|| unsupported("device does not support segmented brightness"))?;

        let value = (percent as u32).max(min).min(max);

        self.control_device(
            device,
            cap,
            json!({
                "segment": vec![segment],
                "brightness": value,
            }),
        )
        .await
    }
}

/// Build the control value for a musicMode capability. sensitivity is clamped
/// to the documented 0-100 range; rgb is included only when a color is supplied,
/// since the platform API rejects an explicit null rgb.
fn music_mode_value(
    mode: JsonValue,
    sensitivity: u8,
    auto_color: bool,
    rgb: Option<u32>,
) -> JsonValue {
    let mut value = json!({
        "musicMode": mode,
        "sensitivity": sensitivity.min(100),
        "autoColor": if auto_color { 1 } else { 0 },
    });
    if let Some(rgb) = rgb {
        value["rgb"] = rgb.into();
    }
    value
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn music_mode_value_rgb() {
        // autoColor on, no color: rgb is irrelevant and must be omitted, because
        // the platform API rejects an explicit null rgb.
        assert_eq!(
            music_mode_value(json!(5), 80, true, None),
            json!({"musicMode": 5, "sensitivity": 80, "autoColor": 1})
        );

        // autoColor off with a color: rgb is included, and sensitivity is
        // clamped to the documented max of 100.
        assert_eq!(
            music_mode_value(json!(5), 200, false, Some(0x0000ff)),
            json!({"musicMode": 5, "sensitivity": 100, "autoColor": 0, "rgb": 255})
        );
    }
}
