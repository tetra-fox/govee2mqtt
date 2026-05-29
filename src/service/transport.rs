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
use crate::service::state::{StateHandle, Transport};
use govee_api::ble::{
    Base64HexBytes, SetDevicePower, SetHumidifierMode, SetHumidifierNightlightParams,
};
use govee_api::lan_api::{DeviceStatus as LanDeviceStatus, LanDevice};
use govee_api::platform_api::DeviceCapability;
use serde_json::Value as JsonValue;
use tokio::time::{Duration, Instant, sleep};

pub(crate) async fn power_on_generic(
    state: &StateHandle,
    device: &Device,
    on: bool,
) -> anyhow::Result<Transport> {
    if let Some(lan_dev) = &device.lan_device {
        log::debug!("Using LAN API to set {device} power state");
        lan_dev.send_turn(on).await?;
        poll_lan_api(state, lan_dev, |status| status.on == on).await?;
        return Ok(Transport::Lan);
    }

    if power_via_ble(state, device, on).await {
        return Ok(Transport::Ble);
    }

    if device.iot_api_supported()
        && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::debug!("Using IoT API to set {device} power state");
        iot.set_power_state(&info.entry, on).await?;
        return Ok(Transport::Iot);
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::debug!("Using Platform API to set {device} power state");
        client.set_power_state(info, on).await?;
        return Ok(Transport::Platform);
    }

    anyhow::bail!("Unable to control power state for {device}");
}

/// Try to set power over direct BLE. Returns true on success; on any failure (no
/// BLE client, no BLE address, connect/handshake/write error) it logs and returns
/// false so the caller falls through to the cloud transports. The BLE client is
/// only present when a Bluetooth adapter was found at startup, so hosts without
/// one never reach a connect attempt here.
async fn power_via_ble(state: &StateHandle, device: &Device, on: bool) -> bool {
    let Some(ble) = state.get_ble_client().await else {
        return false;
    };
    let Some(addr) = device.ble_address() else {
        return false;
    };
    // SetDevicePower is registered under the generic light codec.
    let frame = match Base64HexBytes::encode_for_sku("Generic:Light", &SetDevicePower { on }) {
        Ok(frame) => frame,
        Err(err) => {
            log::warn!("BLE power encode for {device} failed: {err:#}");
            return false;
        }
    };
    log::debug!("Using BLE to set {device} power state");
    state.notify_frame(
        &device.id,
        crate::service::state::FrameDirection::Out,
        crate::service::state::FrameTransport::Ble,
        hex_pretty(frame.bytes()),
    );
    // The connection is kept warm and auto-released after an idle period (see
    // BleClient::send_frames), so bursts reuse one session instead of
    // re-handshaking per command.
    match ble.send_frames(addr, &[frame.bytes().to_vec()]).await {
        Ok(()) => true,
        Err(err) => {
            log::warn!("BLE power for {device} failed ({err:#}); falling through to cloud");
            false
        }
    }
}

/// Apply a socket turn command to the held outlet bits. `outlet == 15` is the
/// "all outlets" form used by the SocketController for whole-device power, so
/// it sets or clears the low `count` bits together; any other index targets
/// just that bit. Returns the new bitfield.
fn apply_outlet_command(prior: u8, count: u8, outlet: u8, on: bool) -> u8 {
    if outlet == 15 {
        let mask = (1u8 << count).saturating_sub(1);
        if on { prior | mask } else { prior & !mask }
    } else {
        let bit = 1u8 << outlet;
        if on { prior | bit } else { prior & !bit }
    }
}

/// Render a BLE frame as space-separated lowercase hex for the inspector.
/// One-line, no prefixes; the ui breaks lines on its own.
fn hex_pretty(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{b:02x}"));
    }
    out
}

pub(crate) async fn light_power_on_generic(
    state: &StateHandle,
    device: &Device,
    on: bool,
) -> anyhow::Result<Transport> {
    let instance_name = device
        .get_light_power_toggle_instance_name()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Don't know how to toggle just the light portion of {device}. \
                 Please share the device metadata and state if you report this issue"
            )
        })?;

    if let Some(lan_dev) = &device.lan_device {
        log::debug!("Using LAN API to set {device} light power state");
        lan_dev.send_turn(on).await?;
        poll_lan_api(state, lan_dev, |status| status.on == on).await?;
        return Ok(Transport::Lan);
    }

    if device.iot_api_supported()
        && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::debug!("Using IoT API to set {device} light power state");
        iot.set_power_state(&info.entry, on).await?;
        return Ok(Transport::Iot);
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::debug!("Using Platform API to set {device} light {instance_name} state");
        client.set_toggle_state(info, instance_name, on).await?;
        return Ok(Transport::Platform);
    }

    anyhow::bail!("Unable to control light power state for {device}");
}

pub(crate) async fn set_brightness_generic(
    state: &StateHandle,
    device: &Device,
    percent: u8,
) -> anyhow::Result<Transport> {
    if let Some(lan_dev) = &device.lan_device {
        log::debug!("Using LAN API to set {device} brightness");
        lan_dev.send_brightness(percent).await?;
        poll_lan_api(state, lan_dev, |status| status.brightness == percent).await?;
        return Ok(Transport::Lan);
    }

    if device.iot_api_supported()
        && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::debug!("Using IoT API to set {device} brightness");
        iot.set_brightness(&info.entry, percent).await?;
        return Ok(Transport::Iot);
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::debug!("Using Platform API to set {device} brightness");
        client.set_brightness(info, percent).await?;
        return Ok(Transport::Platform);
    }
    anyhow::bail!("Unable to control brightness for {device}");
}

pub(crate) async fn set_color_temperature_generic(
    state: &StateHandle,
    device: &Device,
    kelvin: u32,
) -> anyhow::Result<Transport> {
    if let Some(lan_dev) = &device.lan_device {
        log::debug!("Using LAN API to set {device} color temperature");
        lan_dev.send_color_temperature_kelvin(kelvin).await?;
        poll_lan_api(state, lan_dev, |status| {
            status.color_temperature_kelvin == kelvin
        })
        .await?;
        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_active_scene(None);
        return Ok(Transport::Lan);
    }

    if device.iot_api_supported()
        && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::debug!("Using IoT API to set {device} color temperature");
        iot.set_color_temperature(&info.entry, kelvin).await?;
        return Ok(Transport::Iot);
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::debug!("Using Platform API to set {device} color temperature");
        client.set_color_temperature(info, kelvin).await?;
        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_active_scene(None);
        return Ok(Transport::Platform);
    }
    anyhow::bail!("Unable to control color temperature for {device}");
}

pub(crate) async fn set_color_rgb_generic(
    state: &StateHandle,
    device: &Device,
    r: u8,
    g: u8,
    b: u8,
) -> anyhow::Result<Transport> {
    if let Some(lan_dev) = &device.lan_device {
        let color = govee_api::lan_api::DeviceColor { r, g, b };
        log::debug!("Using LAN API to set {device} color");
        lan_dev.send_color_rgb(color).await?;
        poll_lan_api(state, lan_dev, |status| status.color == color).await?;
        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_active_scene(None);
        return Ok(Transport::Lan);
    }

    if device.iot_api_supported()
        && let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::debug!("Using IoT API to set {device} color");
        iot.set_color_rgb(&info.entry, r, g, b).await?;
        return Ok(Transport::Iot);
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::debug!("Using Platform API to set {device} color");
        client.set_color_rgb(info, r, g, b).await?;
        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_active_scene(None);
        return Ok(Transport::Platform);
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
) -> anyhow::Result<Transport> {
    // TODO: some plumbing to maintain offline scene controls for preferred-LAN control
    let avoid_platform_api = device.avoid_platform_api();

    if !avoid_platform_api
        && let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::debug!("Using Platform API to set {device} to scene {scene}");
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
        return Ok(Transport::Platform);
    }

    if let Some(lan_dev) = &device.lan_device {
        log::debug!("Using LAN API to set {device} to scene {scene}");
        lan_dev.set_scene_by_name(scene).await?;

        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_active_scene(Some(scene));
        return Ok(Transport::Lan);
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
) -> anyhow::Result<Transport> {
    let value: JsonValue = value.into();

    // Standard capability instances have a full LAN -> IoT -> Platform cascade
    // already wired up in the generic verbs; route through them so a capability
    // write from HA's number/mode entities or the Web UI's entities panel ends
    // up on the same transport as the convenience routes (`/brightness/{n}`
    // etc.). Without this, every standard capability write lands on the
    // platform API by default and races whatever transport the convenience
    // route just chose, producing visible state flap.
    if let Some(t) = dispatch_standard_instance(state, device, &capability.instance, &value).await?
    {
        return Ok(t);
    }

    // IoT-only capabilities (eg: the H6093 projector's controls) aren't known to
    // the platform API. If this SKU+instance has a ptReal frame encoder, send
    // that; otherwise fall through to the platform API.
    if try_iot_capability(state, device, &capability.instance, &value).await? {
        return Ok(Transport::Iot);
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        log::debug!("Using Platform API to send {value:?} control to {device}");
        client.control_device(info, capability, value).await?;
        return Ok(Transport::Platform);
    }

    anyhow::bail!("Unable to use Platform API to control {device}");
}

/// Map a standard capability instance to the matching generic verb so every
/// transport entry point shares the same cascade. Returns `Some(transport)` if
/// we recognized and dispatched, `None` if the instance falls through to the
/// ptReal / platform fallback.
async fn dispatch_standard_instance(
    state: &StateHandle,
    device: &Device,
    instance: &str,
    value: &JsonValue,
) -> anyhow::Result<Option<Transport>> {
    match instance {
        "powerSwitch" => {
            let on = value
                .as_bool()
                .or_else(|| value.as_i64().map(|v| v != 0))
                .or_else(|| value.as_u64().map(|v| v != 0))
                .ok_or_else(|| anyhow::anyhow!("powerSwitch value {value:?} is not bool/int"))?;
            Ok(Some(power_on_generic(state, device, on).await?))
        }
        "brightness" => {
            let pct: u8 = value
                .as_i64()
                .or_else(|| value.as_u64().map(|v| v as i64))
                .and_then(|v| u8::try_from(v).ok())
                .ok_or_else(|| anyhow::anyhow!("brightness value {value:?} is not 0..=255"))?;
            Ok(Some(set_brightness_generic(state, device, pct).await?))
        }
        "colorRgb" => {
            let packed = value
                .as_i64()
                .or_else(|| value.as_u64().map(|v| v as i64))
                .ok_or_else(|| anyhow::anyhow!("colorRgb value {value:?} is not an integer"))?;
            let r = ((packed >> 16) & 0xff) as u8;
            let g = ((packed >> 8) & 0xff) as u8;
            let b = (packed & 0xff) as u8;
            Ok(Some(set_color_rgb_generic(state, device, r, g, b).await?))
        }
        "colorTemperatureK" => {
            let kelvin: u32 = value
                .as_u64()
                .or_else(|| value.as_i64().and_then(|v| u64::try_from(v).ok()))
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| anyhow::anyhow!("colorTemperatureK value {value:?} is not a u32"))?;
            Ok(Some(
                set_color_temperature_generic(state, device, kelvin).await?,
            ))
        }
        _ => Ok(None),
    }
}

/// Try to send a control for `instance` as an IoT ptReal frame. Returns
/// `Ok(true)` if the SKU+instance has a frame encoder and the command was sent,
/// `Ok(false)` if it isn't a frame-encoded instance (caller falls back to the
/// platform API). The (sku, instance) -> frames mapping lives entirely in the
/// `ble` layer's encoder registry, so this dispatch stays device-agnostic.
pub(crate) async fn try_iot_capability(
    state: &StateHandle,
    device: &Device,
    instance: &str,
    value: &JsonValue,
) -> anyhow::Result<bool> {
    // Aurora/stars controls share one write blob, so they read the held state,
    // mutate one field, and re-send the whole frame. The blob carries the whole
    // aurora/laser state, so we must start from the device's current state: if
    // we haven't got it yet, seed it from the app's stored common-datas.
    let mut blob_state = seeded_aurora_laser_state(state, device).await;
    if govee_api::ble::projector_apply_blob_field(instance, value, &mut blob_state) {
        log::debug!("Using IoT API to set {device} {instance} = {value:?}");
        send_iot_frame(state, device, &blob_state).await?;
        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_aurora_laser_state(blob_state);
        // The device doesn't report these back, so publish our held state to HA
        // ourselves; otherwise the entities stay "unknown".
        state.notify_of_state_change(&device.id).await.ok();
        return Ok(true);
    }

    // Auto-off enable/stop-sound/minutes likewise share one frame.
    let mut auto_off = device.auto_off_state();
    if govee_api::ble::projector_apply_auto_off_field(instance, value, &mut auto_off) {
        send_iot_frame(state, device, &auto_off).await?;
        state
            .device_mut(&device.sku, &device.id)
            .await
            .set_auto_off_state(auto_off);
        state.notify_of_state_change(&device.id).await.ok();
        return Ok(true);
    }

    // Standalone IoT-framed capabilities (eg: settings toggles). Routed by
    // the FamilyModule registry: any family that owns this (sku, instance)
    // returns the base64 frames; None falls through to the platform API below.
    let Some(frames) = govee_api::ble::encode_capability(&device.sku, instance, value) else {
        return Ok(false);
    };
    let frames = frames?;

    let iot = state
        .get_iot_client()
        .await
        .ok_or_else(|| anyhow::anyhow!("IoT client unavailable for {device} {instance}"))?;
    let info = device
        .undoc_device_info
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no IoT device metadata for {device} {instance}"))?;

    log::debug!("Using IoT API to set {device} {instance} = {value:?}");
    iot.send_real(&info.entry, frames).await?;

    // These settings toggles aren't echoed by the device or stored in
    // common-datas, so the only state HA can show is what we just wrote. Record
    // it and publish, otherwise the entity stays unknown even after control.
    if let Some(on) = value.as_bool().or_else(|| value.as_i64().map(|v| v != 0)) {
        let recorded = state
            .device_mut(&device.sku, &device.id)
            .await
            .record_projector_setting(instance, on);
        if recorded {
            state.notify_of_state_change(&device.id).await.ok();
        }
    }
    // H5082 countdowns: stamp the just-written preset onto the held
    // countdown map so HA's state-topic readback reflects the new value
    // before the device's next status broadcast arrives (otherwise the
    // Number entity visually bounces back to whatever the previous
    // broadcast carried).
    if let Some(c) = govee_api::ble::socket::record_optimistic_write(instance, value) {
        state
            .device_mut(&device.sku, &device.id)
            .await
            .record_h5082_countdown(c);
        state.notify_of_state_change(&device.id).await.ok();
    }
    Ok(true)
}

/// Encode a typed command for this SKU and send it over IoT as a ptReal frame.
/// Used by the controls that share a state-carrying frame (aurora/laser blob,
/// auto-off): the caller reads held state, mutates one field, sends here, then
/// writes the mutated state back. Those value bytes aren't fully recoverable
/// from status frames, so that write-back is what keeps held state correct
/// between edits.
async fn send_iot_frame<T: 'static>(
    state: &StateHandle,
    device: &Device,
    value: &T,
) -> anyhow::Result<()> {
    let command = Base64HexBytes::encode_for_sku(&device.sku, value)?;
    let iot = state
        .get_iot_client()
        .await
        .ok_or_else(|| anyhow::anyhow!("IoT client unavailable for {device}"))?;
    let info = device
        .undoc_device_info
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no IoT device metadata for {device}"))?;

    log::debug!("Using IoT API to send a ptReal frame to {device}");
    iot.send_real(&info.entry, command.base64()).await?;
    Ok(())
}

/// Fetch the app's stored common-datas for this device, merge it into the held
/// aurora/laser state, and return the seeded state. None if the SKU isn't seeded
/// from common-datas, there's no undoc client, no stored record, or the read
/// failed. The SKU's (bizType, bizKey) comes from the projector module, so the
/// device-specific knowledge stays there and the SKU gate runs before any HTTP.
///
/// This runs even after status refinement (aa 11/34) has created a held state:
/// refinement only carries the layer on/off bits, while the brightness, colors,
/// effect and speeds come only from common-datas. We take those from common-datas
/// and keep the already-refined on/off bits (the device reports those, and
/// common-datas can lag), so the merged state has both.
async fn try_seed_aurora_laser_state(
    state: &StateHandle,
    device: &Device,
) -> Option<govee_api::ble::SetAuroraLaser> {
    let (biz_type, biz_key) = govee_api::ble::common_datas_seed(&device.sku, &device.id)?;
    let undoc = state.get_undoc_client().await?;
    match undoc.get_common_datas(biz_type, &biz_key).await {
        Ok(Some(json)) => {
            let mut seeded = govee_api::ble::SetAuroraLaser::from_common_datas(&json);
            let mut dev = state.device_mut(&device.sku, &device.id).await;
            if let Some(held) = &dev.aurora_laser_state {
                seeded.aurora_on = held.aurora_on;
                seeded.laser_on = held.laser_on;
            }
            dev.set_aurora_laser_state(seeded.clone());
            dev.mark_aurora_laser_seeded();
            log::debug!("{device}: seeded aurora/laser state from common-datas");
            Some(seeded)
        }
        Ok(None) => {
            log::warn!("{device}: no common-datas record to seed aurora/laser state");
            None
        }
        Err(err) => {
            log::warn!("{device}: failed to read common-datas to seed state: {err:#}");
            None
        }
    }
}

/// Return the device's held aurora+laser state, seeding it from common-datas if it
/// hasn't been seeded yet. A single-field edit re-sends the whole frame, so it
/// must start from the device's real current state including the brightness and
/// colors that only common-datas carries. If the read fails or there's no record,
/// fall back to whatever we hold so control still works (it just may not preserve
/// fields the user hasn't set through us).
async fn seeded_aurora_laser_state(
    state: &StateHandle,
    device: &Device,
) -> govee_api::ble::SetAuroraLaser {
    if device.aurora_laser_seeded {
        return device.aurora_laser_state();
    }
    try_seed_aurora_laser_state(state, device)
        .await
        .unwrap_or_else(|| device.aurora_laser_state())
}

/// Seed a projector's held aurora/laser state from common-datas at poll time, so
/// the entities whose values aren't in the platform or IoT status (relative
/// brightness, color mode, effect, flow) show their real values at startup, and so
/// a layer toggled on carries its real brightness and colors instead of being
/// invisible. One-shot: gated on `aurora_laser_seeded`, not on whether a held
/// state exists, since status refinement creates a held state before this runs.
pub(crate) async fn ensure_projector_state_seeded(state: &StateHandle, device: &Device) {
    if device.aurora_laser_seeded {
        return;
    }
    if try_seed_aurora_laser_state(state, device).await.is_some() {
        // Publish the seeded values so the synthesized entities populate; a failed
        // notify is retried on the next poll.
        state.notify_of_state_change(&device.id).await.ok();
    }
}

/// Switch one outlet of a Wi-Fi smart plug/switch. `outlet` is the zero-based
/// outlet index, or 15 for all outlets. Tries IoT first (REST relay for shared,
/// direct MQTT for owned, decided in
/// [`crate::service::iot::IotClient::set_socket_power`]); falls back to the
/// platform API's per-outlet `socketToggleN` capability when no IoT device info
/// is available. LAN and BLE don't fit here: their power command is
/// device-wide, not per-outlet.
///
/// Note: unlike the generic verbs we do not gate this on `iot_api_supported`.
/// Per-outlet control is IoT-only by construction (the platform fallback below
/// is documented as buggy for these SKUs), so a missing IoT-support quirk flag
/// must not silently route us to the broken fallback.
pub(crate) async fn socket_turn(
    state: &StateHandle,
    device: &Device,
    outlet: u8,
    on: bool,
) -> anyhow::Result<Transport> {
    if let Some(iot) = state.get_iot_client().await
        && let Some(info) = &device.undoc_device_info
    {
        log::debug!("Using IoT API to set {device} outlet {outlet} -> {on}");
        iot.set_socket_power(&info.entry, outlet, on).await?;
        // optimistically reflect the command in the held outlet bits so the
        // ui paints the new state before the device's status reply arrives.
        // the subscriber overwrites this when the response (or any later
        // status broadcast) lands. only meaningful for multi-outlet sockets;
        // single-outlet path leaves the bits alone since they aren't read.
        if let Some(count) = device.socket_outlet_count() {
            let prior = device.socket_outlet_bits.unwrap_or(0);
            let next = apply_outlet_command(prior, count, outlet, on);
            state
                .device_mut(&device.sku, &device.id)
                .await
                .set_socket_outlet_bits(next);
            state.notify_of_state_change(&device.id).await.ok();
        }
        return Ok(Transport::IotSocket);
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(http_dev) = &device.http_device_info
    {
        // The platform API exposes each outlet as its own toggle capability,
        // 1-indexed in the instance name (socketToggle1 = outlet 0). The
        // outlet=15 broadcast is IoT-only; no per-capability equivalent on the
        // platform API.
        let instance = format!("socketToggle{}", outlet + 1);
        if http_dev.capability_by_instance(&instance).is_some() {
            log::debug!("Using Platform API to set {device} {instance} -> {on}");
            client.set_toggle_state(http_dev, &instance, on).await?;
            return Ok(Transport::Platform);
        }
    }

    anyhow::bail!("Unable to control outlet {outlet} for {device}");
}

/// Set a fan's speed via the platform-API workMode capability. `work_mode`
/// is the value of the "FanSpeed" mode (parsed by the caller from the
/// device's workMode enum); `speed` is the integer speed level. Per the
/// lasswellt govee-homeassistant protocol reference, Govee fans expose
/// speed as `workMode={FanSpeed value}, modeValue={speed}` on a single
/// workMode capability. We do not currently have a BLE codec for fan
/// frames; if/when one is added it can layer in front of this fallback
/// like SetHumidifierMode does for humidifiers.
pub(crate) async fn fan_set_speed(
    state: &StateHandle,
    device: &Device,
    work_mode: i64,
    speed: i64,
) -> anyhow::Result<Transport> {
    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        client.set_work_mode(info, work_mode, speed).await?;
        return Ok(Transport::Platform);
    }
    anyhow::bail!("Unable to control fan speed for {device}: no platform-API client");
}

/// Set a humidifier work mode and parameter. Tries the BLE-encoded
/// SetHumidifierMode over IoT first, then falls back to the platform API's
/// work-mode capability.
pub(crate) async fn humidifier_set_parameter(
    state: &StateHandle,
    device: &Device,
    work_mode: i64,
    value: i64,
) -> anyhow::Result<Transport> {
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
        return Ok(Transport::Iot);
    }

    if let Some(client) = state.get_platform_client().await
        && let Some(info) = &device.http_device_info
    {
        client.set_work_mode(info, work_mode, value).await?;
        return Ok(Transport::Platform);
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
        log::debug!("Using IoT API to set {device} color");
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

    log::debug!("Polling {device} to get latest state after control");
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
