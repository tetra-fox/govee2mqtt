//! Per-device-type control dispatch: which wire command a control verb maps to
//! for a given device type. The transport each command goes out on (LAN vs IoT
//! vs platform) is a separate decision, in [`crate::service::transport`].
//!
//! Most devices control the same way, so the default methods of
//! [`DeviceController`] just call the generic transport cascade.
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
//! another branch through the transport functions.

use crate::service::device::Device;
use crate::service::state::{StateHandle, Transport};
use crate::service::transport;
use async_trait::async_trait;
use govee_api::platform_api::DeviceType;

/// Control behavior for one device type. The default methods are the generic
/// transport cascade; implementors override the verbs that map onto a different
/// command for their device type.
///
/// The `Ok` payload of each method is the name of the transport that accepted
/// the command (e.g. "LAN", "BLE", "IoT", "PLATFORM"), so the wrapper layer
/// can record which transport won in the command-history ring.
#[async_trait]
pub trait DeviceController: Send + Sync {
    async fn power_on(
        &self,
        state: &StateHandle,
        device: &Device,
        on: bool,
    ) -> anyhow::Result<Transport> {
        transport::power_on_generic(state, device, on).await
    }

    async fn light_power_on(
        &self,
        state: &StateHandle,
        device: &Device,
        on: bool,
    ) -> anyhow::Result<Transport> {
        transport::light_power_on_generic(state, device, on).await
    }

    async fn set_brightness(
        &self,
        state: &StateHandle,
        device: &Device,
        percent: u8,
    ) -> anyhow::Result<Transport> {
        transport::set_brightness_generic(state, device, percent).await
    }

    async fn set_color_rgb(
        &self,
        state: &StateHandle,
        device: &Device,
        r: u8,
        g: u8,
        b: u8,
    ) -> anyhow::Result<Transport> {
        transport::set_color_rgb_generic(state, device, r, g, b).await
    }

    async fn set_color_temperature(
        &self,
        state: &StateHandle,
        device: &Device,
        kelvin: u32,
    ) -> anyhow::Result<Transport> {
        transport::set_color_temperature_generic(state, device, kelvin).await
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
    ) -> anyhow::Result<Transport> {
        if transport::try_set_nightlight(state, device, |p| p.on = on).await? {
            return Ok(Transport::Iot);
        }
        transport::light_power_on_generic(state, device, on).await
    }

    async fn set_brightness(
        &self,
        state: &StateHandle,
        device: &Device,
        percent: u8,
    ) -> anyhow::Result<Transport> {
        if transport::try_set_nightlight(state, device, |p| {
            p.brightness = percent;
            p.on = true;
        })
        .await?
        {
            return Ok(Transport::Iot);
        }
        transport::set_brightness_generic(state, device, percent).await
    }

    async fn set_color_rgb(
        &self,
        state: &StateHandle,
        device: &Device,
        r: u8,
        g: u8,
        b: u8,
    ) -> anyhow::Result<Transport> {
        if transport::try_set_nightlight(state, device, |p| {
            p.r = r;
            p.g = g;
            p.b = b;
            p.on = true;
        })
        .await?
        {
            return Ok(Transport::Iot);
        }
        transport::set_color_rgb_generic(state, device, r, g, b).await
    }
}

/// A Wi-Fi smart plug/switch sends its power as a packed `turn` value through
/// the IoT API; the rest of the verbs don't apply.
struct SocketController;

#[async_trait]
impl DeviceController for SocketController {
    async fn power_on(
        &self,
        state: &StateHandle,
        device: &Device,
        on: bool,
    ) -> anyhow::Result<Transport> {
        // 15 addresses all outlets, the form the app uses for a single-outlet
        // plug. Shared vs owned transport is handled downstream.
        transport::socket_turn(state, device, 15, on).await?;
        Ok(Transport::Iot)
    }
}

pub fn controller_for(device: &Device) -> Box<dyn DeviceController> {
    match device.device_type() {
        DeviceType::Humidifier | DeviceType::Dehumidifier => Box::new(HumidifierController),
        DeviceType::Socket => Box::new(SocketController),
        _ => Box::new(GenericController),
    }
}
