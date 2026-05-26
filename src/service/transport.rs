//! Transport selection: which wire protocol carries a given control command.
//!
//! Each control verb runs a cascade over the transports a device exposes,
//! taking the first one available. For most verbs that order is LAN, then IoT
//! MQTT, then the platform REST API. The LAN API only carries on/off,
//! brightness, color, color temperature, and scenes, so anything richer (a
//! numeric work-mode parameter, a generic capability) skips LAN and goes
//! straight to IoT or the platform API.
//!
//! These are free functions taking `&StateHandle` rather than methods on
//! `State`, matching the per-device-type controllers in
//! [`crate::service::control`]: control.rs decides *which verb* to run for a
//! device type, transport.rs decides *which wire* each verb goes out on. The
//! thin `device_*` wrappers that the rest of the app calls stay on `State` and
//! delegate in here.

use crate::service::device::Device;
use crate::service::state::StateHandle;
use govee_api::ble::{Base64HexBytes, SetHumidifierMode, SetHumidifierNightlightParams};
use govee_api::lan_api::{DeviceStatus as LanDeviceStatus, LanDevice};
use govee_api::platform_api::DeviceCapability;
use serde_json::Value as JsonValue;
use tokio::time::{Duration, Instant, sleep};

pub(crate) async fn power_on_generic(
    state: &StateHandle,
    device: &Device,
    on: bool,
) -> anyhow::Result<()> {
    if let Some(lan_dev) = &device.lan_device {
        log::info!("Using LAN API to set {device} power state");
        lan_dev.send_turn(on).await?;
        poll_lan_api(state, lan_dev, |status| status.on == on).await?;
        return Ok(());
    }

    if device.iot_api_supported()
        && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::info!("Using IoT API to set {device} power state");
        iot.set_power_state(&info.entry, on).await?;
        return Ok(());
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::info!("Using Platform API to set {device} power state");
        client.set_power_state(info, on).await?;
        return Ok(());
    }

    anyhow::bail!("Unable to control power state for {device}");
}

pub(crate) async fn light_power_on_generic(
    state: &StateHandle,
    device: &Device,
    on: bool,
) -> anyhow::Result<()> {
    let instance_name = device
        .get_light_power_toggle_instance_name()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Don't know how to toggle just the light portion of {device}. \
                 Please share the device metadata and state if you report this issue"
            )
        })?;

    if let Some(lan_dev) = &device.lan_device {
        log::info!("Using LAN API to set {device} light power state");
        lan_dev.send_turn(on).await?;
        poll_lan_api(state, lan_dev, |status| status.on == on).await?;
        return Ok(());
    }

    if device.iot_api_supported()
        && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::info!("Using IoT API to set {device} light power state");
        iot.set_power_state(&info.entry, on).await?;
        return Ok(());
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::info!("Using Platform API to set {device} light {instance_name} state");
        client.set_toggle_state(info, instance_name, on).await?;
        return Ok(());
    }

    anyhow::bail!("Unable to control light power state for {device}");
}

pub(crate) async fn set_brightness_generic(
    state: &StateHandle,
    device: &Device,
    percent: u8,
) -> anyhow::Result<()> {
    if let Some(lan_dev) = &device.lan_device {
        log::info!("Using LAN API to set {device} brightness");
        lan_dev.send_brightness(percent).await?;
        poll_lan_api(state, lan_dev, |status| status.brightness == percent).await?;
        return Ok(());
    }

    if device.iot_api_supported()
        && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::info!("Using IoT API to set {device} brightness");
        iot.set_brightness(&info.entry, percent).await?;
        return Ok(());
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::info!("Using Platform API to set {device} brightness");
        client.set_brightness(info, percent).await?;
        return Ok(());
    }
    anyhow::bail!("Unable to control brightness for {device}");
}

pub(crate) async fn set_color_temperature_generic(
    state: &StateHandle,
    device: &Device,
    kelvin: u32,
) -> anyhow::Result<()> {
    if let Some(lan_dev) = &device.lan_device {
        log::info!("Using LAN API to set {device} color temperature");
        lan_dev.send_color_temperature_kelvin(kelvin).await?;
        poll_lan_api(state, lan_dev, |status| {
            status.color_temperature_kelvin == kelvin
        })
        .await?;
        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_active_scene(None);
        return Ok(());
    }

    if device.iot_api_supported()
        && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::info!("Using IoT API to set {device} color temperature");
        iot.set_color_temperature(&info.entry, kelvin).await?;
        return Ok(());
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::info!("Using Platform API to set {device} color temperature");
        client.set_color_temperature(info, kelvin).await?;
        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_active_scene(None);
        return Ok(());
    }
    anyhow::bail!("Unable to control color temperature for {device}");
}

pub(crate) async fn set_color_rgb_generic(
    state: &StateHandle,
    device: &Device,
    r: u8,
    g: u8,
    b: u8,
) -> anyhow::Result<()> {
    if let Some(lan_dev) = &device.lan_device {
        let color = govee_api::lan_api::DeviceColor { r, g, b };
        log::info!("Using LAN API to set {device} color");
        lan_dev.send_color_rgb(color).await?;
        poll_lan_api(state, lan_dev, |status| status.color == color).await?;
        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_active_scene(None);
        return Ok(());
    }

    if device.iot_api_supported()
        && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::info!("Using IoT API to set {device} color");
        iot.set_color_rgb(&info.entry, r, g, b).await?;
        return Ok(());
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::info!("Using Platform API to set {device} color");
        client.set_color_rgb(info, r, g, b).await?;
        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_active_scene(None);
        return Ok(());
    }
    anyhow::bail!("Unable to control color for {device}");
}

/// Set a scene. Unlike the other verbs this prefers the platform API, because
/// the LAN scene path tunnels a BLE-encoded scene packet via `ptReal` and is
/// the fallback only when the platform API is unavailable or quirked off.
pub(crate) async fn device_set_scene(
    state: &StateHandle,
    device: &Device,
    scene: &str,
) -> anyhow::Result<()> {
    // TODO: some plumbing to maintain offline scene controls for preferred-LAN control
    let avoid_platform_api = device.avoid_platform_api();

    if !avoid_platform_api
        && let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::info!("Using Platform API to set {device} to scene {scene}");
        client
            .set_scene_by_name_with_music(
                info,
                scene,
                device.music_sensitivity(),
                device.music_auto_color(),
            )
            .await?;
        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_active_scene(Some(scene));
        return Ok(());
    }

    if let Some(lan_dev) = &device.lan_device {
        log::info!("Using LAN API to set {device} to scene {scene}");
        lan_dev.set_scene_by_name(scene).await?;

        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_active_scene(Some(scene));
        return Ok(());
    }

    anyhow::bail!("Unable to set scene for {device}");
}

/// Generic capability control (numeric ranges, enum modes, toggles). The LAN
/// API has no command for these, so this goes straight to the platform API
/// with no fallback.
pub(crate) async fn device_control<V: Into<JsonValue>>(
    state: &StateHandle,
    device: &Device,
    capability: &DeviceCapability,
    value: V,
) -> anyhow::Result<()> {
    let value: JsonValue = value.into();
    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::info!("Using Platform API to send {value:?} control to {device}");
        client.control_device(info, capability, value).await?;
        return Ok(());
    }

    anyhow::bail!("Unable to use Platform API to control {device}");
}

/// Switch one outlet of a Wi-Fi smart plug/switch. `outlet` is the zero-based
/// outlet index, or 15 for all outlets. Always IoT; the REST relay vs direct
/// MQTT choice (shared vs owned) is made downstream in
/// [`crate::service::iot::IotClient::set_socket_power`].
pub(crate) async fn socket_turn(
    state: &StateHandle,
    device: &Device,
    outlet: u8,
    on: bool,
) -> anyhow::Result<()> {
    let info = device
        .undoc_device_info
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("{device} has no undoc metadata; cannot control socket"))?;
    let iot = state
        .get_iot_client()
        .await
        .ok_or_else(|| anyhow::anyhow!("IoT client unavailable for {device}"))?;

    log::info!("Using IoT API to set {device} outlet {outlet} -> {on}");
    iot.set_socket_power(&info.entry, outlet, on).await
}

/// Set a humidifier work mode and parameter. Tries the BLE-encoded
/// SetHumidifierMode over IoT first, then falls back to the platform API's
/// work-mode capability.
pub(crate) async fn humidifier_set_parameter(
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

/// Encode the humidifier nightlight params and send them over IoT. Returns true
/// if the command was sent, false if the device has no IoT transport or undoc
/// metadata (so the caller can fall back to the generic cascade).
pub(crate) async fn try_set_nightlight<F: Fn(&mut SetHumidifierNightlightParams)>(
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

/// After issuing a control command, poll for fresh state if the device has no
/// transport that pushes its state back to us. LAN-polled and IoT-pushed
/// devices update on their own, so only platform-API-only devices need this.
pub(crate) async fn poll_after_control(state: &StateHandle, id: String) {
    let Some(device) = state.device_by_id(&id).await else {
        return;
    };

    let iot_available = state.get_iot_client().await.is_some();

    if device.pollable_via_iot() && iot_available {
        return;
    }
    if device.pollable_via_lan() {
        return;
    }

    // Add a slight delay, as the status returned
    // by the platform API isn't guaranteed to be
    // coherent with the command we just issued
    // right away :-/
    sleep(Duration::from_secs(5)).await;

    log::info!("Polling {device} to get latest state after control");
    if let Err(err) = state.poll_platform_api(&device).await {
        log::error!("Polling {device} failed: {err:#}");
    }
}

/// Poll a LAN device's status until `acceptor` accepts it or a 5s deadline
/// passes, updating the cached status each round. Used by the cascade verbs to
/// confirm a LAN command took effect.
async fn poll_lan_api<F: Fn(&LanDeviceStatus) -> bool>(
    state: &StateHandle,
    device: &LanDevice,
    acceptor: F,
) -> anyhow::Result<()> {
    match state.get_lan_client().await {
        Some(client) => {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() <= deadline {
                let status = client.query_status(device).await?;
                let accepted = (acceptor)(&status);
                state
                    .device_mut(&device.sku, &device.device)
                    .await
                    .set_lan_device_status(status);
                if accepted {
                    break;
                }
                sleep(Duration::from_millis(100)).await;
            }
            state.notify_of_state_change(&device.device).await?;
            Ok(())
        }
        None => anyhow::bail!("no lan client"),
    }
}
