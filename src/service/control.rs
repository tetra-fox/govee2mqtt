//! Per-device-type control routing.
//!
//! Most devices control the same way: try the LAN API, then the IoT MQTT API,
//! then the platform REST API, taking the first transport that is available.
//! That generic cascade lives in the default methods of [`DeviceController`].
//!
//! Some device types map a control verb onto a different wire command. A
//! humidifier's "light" is its nightlight, set through a BLE nightlight packet
//! rather than a colorwc/brightness command. A Wi-Fi socket's power is a packed
//! `turn` value (outlet select nibble + on/off nibble), not a plain boolean.
//! Those types override only the verbs that differ and fall back to the generic
//! cascade for the rest.
//!
//! [`controller_for`] picks the implementation from the device type, so adding a
//! device type with bespoke control means adding an impl here, not threading
//! another branch through the state methods.

use crate::service::device::Device;
use crate::service::state::StateHandle;
use async_trait::async_trait;
use govee_api::ble::{Base64HexBytes, SetHumidifierMode, SetHumidifierNightlightParams};
use govee_api::platform_api::DeviceType;

/// Control behavior for one device type. The default methods are the generic
/// transport cascade; implementors override the verbs that map onto a different
/// command for their device type.
#[async_trait]
pub trait DeviceController: Send + Sync {
    async fn power_on(&self, state: &StateHandle, device: &Device, on: bool) -> anyhow::Result<()> {
        state.power_on_generic(device, on).await
    }

    async fn light_power_on(
        &self,
        state: &StateHandle,
        device: &Device,
        on: bool,
    ) -> anyhow::Result<()> {
        state.light_power_on_generic(device, on).await
    }

    async fn set_brightness(
        &self,
        state: &StateHandle,
        device: &Device,
        percent: u8,
    ) -> anyhow::Result<()> {
        state.set_brightness_generic(device, percent).await
    }

    async fn set_color_rgb(
        &self,
        state: &StateHandle,
        device: &Device,
        r: u8,
        g: u8,
        b: u8,
    ) -> anyhow::Result<()> {
        state.set_color_rgb_generic(device, r, g, b).await
    }

    async fn set_color_temperature(
        &self,
        state: &StateHandle,
        device: &Device,
        kelvin: u32,
    ) -> anyhow::Result<()> {
        state.set_color_temperature_generic(device, kelvin).await
    }
}

/// Lights and anything without bespoke control: pure generic cascade.
struct GenericController;
impl DeviceController for GenericController {}

/// On a humidifier the controllable "light" is its nightlight, set with a BLE
/// nightlight packet. Power/brightness/color route there first, falling back to
/// the generic cascade if the device has no nightlight or no IoT transport.
struct HumidifierController;

#[async_trait]
impl DeviceController for HumidifierController {
    async fn light_power_on(
        &self,
        state: &StateHandle,
        device: &Device,
        on: bool,
    ) -> anyhow::Result<()> {
        if try_set_nightlight(state, device, |p| p.on = on).await? {
            return Ok(());
        }
        state.light_power_on_generic(device, on).await
    }

    async fn set_brightness(
        &self,
        state: &StateHandle,
        device: &Device,
        percent: u8,
    ) -> anyhow::Result<()> {
        if try_set_nightlight(state, device, |p| {
            p.brightness = percent;
            p.on = true;
        })
        .await?
        {
            return Ok(());
        }
        state.set_brightness_generic(device, percent).await
    }

    async fn set_color_rgb(
        &self,
        state: &StateHandle,
        device: &Device,
        r: u8,
        g: u8,
        b: u8,
    ) -> anyhow::Result<()> {
        if try_set_nightlight(state, device, |p| {
            p.r = r;
            p.g = g;
            p.b = b;
            p.on = true;
        })
        .await?
        {
            return Ok(());
        }
        state.set_color_rgb_generic(device, r, g, b).await
    }
}

/// A Wi-Fi smart plug/switch sends its power as a packed `turn` value through
/// the IoT API; the rest of the verbs don't apply.
struct SocketController;

#[async_trait]
impl DeviceController for SocketController {
    async fn power_on(&self, state: &StateHandle, device: &Device, on: bool) -> anyhow::Result<()> {
        // 15 addresses all outlets, the form the app uses for a single-outlet
        // plug. Shared vs owned transport is handled downstream.
        state.socket_turn(device, 15, on).await
    }
}

pub fn controller_for(device: &Device) -> Box<dyn DeviceController> {
    match device.device_type() {
        DeviceType::Humidifier | DeviceType::Dehumidifier => Box::new(HumidifierController),
        DeviceType::Socket => Box::new(SocketController),
        _ => Box::new(GenericController),
    }
}

/// Encode the humidifier nightlight params and send them over IoT. Returns true
/// if the command was sent, false if the device has no IoT transport or undoc
/// metadata (so the caller can fall back to the generic cascade).
async fn try_set_nightlight<F: Fn(&mut SetHumidifierNightlightParams)>(
    state: &StateHandle,
    device: &Device,
    apply: F,
) -> anyhow::Result<bool> {
    let mut params: SetHumidifierNightlightParams =
        device.nightlight_state.unwrap_or_default().into();
    (apply)(&mut params);

    if let Ok(command) = Base64HexBytes::encode_for_sku(&device.sku, &params)
        && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::info!("Using IoT API to set {device} color");
        iot.send_real(&info.entry, command.base64()).await?;
        return Ok(true);
    }

    Ok(false)
}

/// Set a humidifier work mode and parameter. Tries the BLE-encoded SetHumidifierMode
/// over IoT first, then falls back to the platform API's work-mode capability.
pub async fn humidifier_set_parameter(
    state: &StateHandle,
    device: &Device,
    work_mode: i64,
    value: i64,
) -> anyhow::Result<()> {
    if let Ok(command) = Base64HexBytes::encode_for_sku(
        &device.sku,
        &SetHumidifierMode {
            mode: work_mode as u8,
            param: value as u8,
        },
    ) && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        iot.send_real(&info.entry, command.base64()).await?;
        return Ok(());
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        client.set_work_mode(info, work_mode, value).await?;
        return Ok(());
    }
    anyhow::bail!("Unable to control humidifier parameter work_mode={work_mode} for {device}");
}
