use crate::UndocApiArguments;
use crate::service::device::Device;
use crate::service::hass::spawn_hass_integration;
use crate::service::http::run_http_server;
use crate::service::iot::start_iot_client;
use crate::service::platform_iot::start_platform_iot;
use crate::service::state::StateHandle;
use crate::version_info::govee_version;
use anyhow::Context;
use chrono::Utc;
use govee_api::lan_api::Client as LanClient;
use govee_api::platform_api::GoveeApiClient;
use govee_api::undoc_api::GoveeUndocumentedApi;
use once_cell::sync::{Lazy, OnceCell};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{Duration, sleep};

pub static POLL_INTERVAL: Lazy<chrono::Duration> = Lazy::new(|| chrono::Duration::seconds(900));

pub const DEFAULT_AVAILABILITY_TIMEOUT_SECS: i64 = 300;

/// How long a device may go without us hearing from it before we treat it as
/// offline. The Govee cloud marks an unreachable device offline within about a
/// minute (measured by unplugging an H6093 and polling the platform online
/// flag), but our state poll is too slow to notice promptly, and IoT replies
/// just stop arriving. This timeout is the silence window after which
/// availability_status reports Missing. The Govee app uses 60s, but it only
/// runs while open; as an always-on daemon we default higher (see
/// DEFAULT_AVAILABILITY_TIMEOUT_SECS) to keep IoT status traffic modest, and
/// let the user tune it via --availability-timeout / GOVEE2MQTT_AVAILABILITY_TIMEOUT.
/// Set once at serve startup; availability_status reads it.
static AVAILABILITY_TIMEOUT: OnceCell<chrono::Duration> = OnceCell::new();

pub fn availability_timeout() -> chrono::Duration {
    *AVAILABILITY_TIMEOUT
        .get()
        .unwrap_or(&chrono::Duration::seconds(
            DEFAULT_AVAILABILITY_TIMEOUT_SECS,
        ))
}

/// How soon we re-send an IoT status request for a device we have no fresh
/// state for. IoT replies are async and can be lost, and poll_iot_api marks the
/// device polled on send, so without a shorter lockout than POLL_INTERVAL a
/// single missed reply would leave the device stale for the full interval.
/// Tied to half the availability timeout so a live device is probed at least
/// twice per window and a lost reply doesn't flip it offline.
pub fn iot_resend_interval() -> chrono::Duration {
    availability_timeout() / 2
}

#[derive(clap::Parser, Debug)]
pub struct ServeCommand {
    /// The port on which the HTTP API will listen
    #[arg(long, default_value_t = 8056)]
    http_port: u16,

    /// Seconds of silence from a device before it is reported offline in home
    /// assistant. Lower means faster offline detection but more frequent IoT
    /// status polling. You may also set this via the
    /// GOVEE2MQTT_AVAILABILITY_TIMEOUT environment variable.
    #[arg(long)]
    availability_timeout: Option<i64>,

    /// Enable direct Bluetooth (BLE) control of owned devices, preferred over the
    /// cloud when a device is in range. Requires a Bluetooth adapter on the host;
    /// if none is found this has no effect and the cloud transports are used.
    #[arg(long, env = "GOVEE2MQTT_ENABLE_BLE")]
    enable_ble: bool,
}

async fn poll_single_device(state: &StateHandle, device: &Device) -> anyhow::Result<()> {
    let now = Utc::now();

    // The H6093 projector's relative brightness, color mode, effect and flow
    // aren't in the platform or IoT status; they come from the app's stored
    // common-datas. Seed the held state from it once so those entities populate
    // at startup instead of staying default until the user changes one.
    crate::service::transport::ensure_projector_state_seeded(state, device).await;

    if device.is_ble_only_device() == Some(true) {
        // We can't poll this device, we have no ble support
        return Ok(());
    }

    // Collect the device status via the LAN API, if possible.
    // This is partially redundant with the LAN discovery task,
    // but the timing of that is not as regular and predictable
    // because it employs exponential backoff.
    // Some Govee devices have bad firmware that will cause the
    // lights to flicker about a minute after polling, so it
    // is desirable to keep polling on a regular basis.
    // <https://github.com/wez/govee2mqtt/issues/250>
    if let Some(lan_device) = &device.lan_device
        && let Some(client) = state.get_lan_client().await
        && let Ok(status) = client.query_status(lan_device).await
    {
        state
            .device_mut(&lan_device.sku, &lan_device.device)
            .await
            .set_lan_device_status(status);
        state.notify_of_state_change(&lan_device.device).await.ok();
    }

    let poll_interval = device.preferred_poll_interval();
    let needs_platform = device.needs_platform_poll();

    // last_polled is when we last sent a request, not when a reply arrived. An
    // IoT reply is async and may be lost, and poll_iot_api marks the device
    // polled on send, so gating the re-send on the full poll_interval would
    // leave a device that missed a single reply stale for the whole interval.
    // For IoT-polled devices gate re-sending on the shorter resend interval so
    // we probe each device at least twice per availability window and it goes
    // offline promptly once it stops answering. Platform polls fetch state
    // synchronously over the rate-limited HTTP API, so they keep the full
    // interval.
    let resend_interval = if needs_platform {
        poll_interval
    } else {
        poll_interval.min(iot_resend_interval())
    };
    let can_update = match &device.last_polled {
        None => true,
        Some(last) => now - last > resend_interval,
    };

    if !can_update {
        return Ok(());
    }

    let device_state = device.device_state();
    let needs_update = match &device_state {
        None => true,
        Some(state) => now - state.updated > poll_interval,
    };

    if !needs_update {
        return Ok(());
    }

    // Don't interrogate via HTTP if we can use the LAN.
    // If we have LAN and the device is stale, it is likely
    // offline and there is little sense in burning up request
    // quota to the platform API for it
    if device.lan_device.is_some() && !needs_platform {
        log::trace!("LAN-available device {device} needs a status update; it's likely offline.");
        return Ok(());
    }

    if !needs_platform && state.poll_iot_api(device).await? {
        return Ok(());
    }

    // Shared devices aren't returned by the platform API (no http_device_info),
    // so poll_platform_api can't reach them. Request status over IoT instead,
    // which routes through the REST relay for shared devices. Without this they
    // never get a device_state and show as unavailable in hass.
    if device.http_device_info.is_none() && state.poll_iot_api(device).await? {
        return Ok(());
    }

    state.poll_platform_api(device).await?;

    Ok(())
}

async fn periodic_state_poll(state: StateHandle) -> anyhow::Result<()> {
    // Wait for the IoT client to connect before the first poll. poll_iot_api
    // publishes a status request and receives the reply on the account topic the
    // IoT subscriber subscribes to on ConnAck; publishing before that
    // subscription is live loses the reply. And poll_iot_api marks the device
    // polled on publish, so a lost first reply isn't retried until the full poll
    // interval (~15min), leaving that device unavailable that whole time. The
    // bound stops a broken IoT connection from stalling polling forever; a
    // LAN/platform-only setup has no IoT client and skips the wait.
    if state.get_iot_client().await.is_some() {
        // ignore the result: on timeout we poll anyway, accepting the race
        let _ = tokio::time::timeout(Duration::from_secs(10), state.wait_for_iot_ready()).await;
    }
    let mut first_pass = true;
    loop {
        for d in state.devices().await {
            // On the first pass, don't spend cloud request quota polling a
            // device that LAN discovery is expected to answer for: leave it to
            // the disco task, which publishes state as devices respond over the
            // LAN. The device stays unavailable until LAN (or, if its LAN never
            // answers, a later cloud poll once lan_device is still unset) gives
            // us state, which is the honest status. A device with no LAN path
            // is polled now over the cloud.
            if first_pass
                && d.lan_device.is_none()
                && d.resolve_quirk().is_some_and(|q| q.lan_api_capable)
            {
                continue;
            }
            if let Err(err) = poll_single_device(&state, &d).await {
                log::error!("while polling {d}: {err:#}");
            }
        }

        // Republish per-device availability after the poll cycle. A successful
        // poll already publishes it via the state-change path, but a device
        // that stops responding never fires a state change, so without this
        // sweep its availability would stay "online" after its state went
        // stale. Publishing the current status for every device here flips
        // those to offline.
        if let Some(hass) = state.get_hass_client().await {
            for d in state.devices().await {
                if !d.is_controllable() {
                    continue;
                }
                if let Err(err) = hass.publish_device_availability(&d, &state).await {
                    log::error!("while publishing availability for {d}: {err:#}");
                }
            }
        }

        first_pass = false;
        sleep(Duration::from_secs(30)).await;
    }
}

/// Log the resolved device set once, with per-device API capabilities and
/// warnings for devices that should be reachable over LAN but didn't respond.
/// Runs off the startup critical path (see the caller) so registration and
/// polling don't block on it.
async fn log_device_summary(state: &StateHandle) {
    log::info!("Devices returned from Govee's APIs");
    for device in state.devices().await {
        log::info!("{device}");
        if let Some(lan) = &device.lan_device {
            log::info!("  LAN API: ip={:?}", lan.ip);
        }
        if let Some(http_info) = &device.http_device_info {
            let kind = &http_info.device_type;
            let rgb = http_info.supports_rgb();
            let bright = http_info.supports_brightness();
            let color_temp = http_info.get_color_temperature_range();
            let segment_rgb = http_info.supports_segmented_rgb();
            log::info!("  Platform API: {kind}. supports_rgb={rgb} supports_brightness={bright}");
            log::info!("                color_temp={color_temp:?} segment_rgb={segment_rgb:?}");
            log::trace!("{http_info:#?}");
        }
        if let Some(undoc) = &device.undoc_device_info {
            let room = &undoc.room_name;
            let supports_iot = undoc.entry.device_ext.device_settings.topic.is_some();
            let ble_only = undoc.entry.device_ext.device_settings.wifi_name.is_none();
            log::info!("  Undoc: room={room:?} supports_iot={supports_iot} ble_only={ble_only}");
            log::trace!("{undoc:#?}");
        }
        if let Some(quirk) = device.resolve_quirk() {
            log::info!("  {quirk:?}");

            // Sanity check for LAN devices: if we don't see an API for it,
            // it may indicate a networking issue
            if quirk.lan_api_capable && device.lan_device.is_none() {
                log::warn!(
                    "  This device should be available via the LAN API, \
                    but didn't respond to probing yet. Possible causes:"
                );
                log::warn!("  1) LAN API needs to be enabled in the Govee Home App.");
                log::warn!("  2) The device is offline.");
                log::warn!("  3) A network configuration issue is preventing communication.");
                log::warn!("  4) The device needs a firmware update before it can enable LAN API.");
                log::warn!(
                    "  5) The hardware version of the device is too old to enable the LAN API."
                );
            }
        } else if device.http_device_info.is_none() {
            log::warn!("  Unknown device type. Cannot map to Home Assistant.");
            if state.get_platform_client().await.is_none() {
                log::warn!(
                    "  Recommendation: configure your Govee API Key so that \
                              metadata can be fetched from Govee"
                );
            }
        }

        log::info!("");
    }
}

async fn enumerate_devices_via_platform_api(
    state: StateHandle,
    client: Option<GoveeApiClient>,
) -> anyhow::Result<()> {
    let client = match client {
        Some(client) => client,
        None => match state.get_platform_client().await {
            Some(client) => client,
            None => return Ok(()),
        },
    };

    log::info!("Querying platform API for device list");
    for info in client.get_devices().await? {
        let mut device = state.device_mut(&info.sku, &info.device).await;
        device.set_http_device_info(info);
    }
    Ok(())
}

async fn enumerate_devices_via_undo_api(
    state: StateHandle,
    client: Option<GoveeUndocumentedApi>,
    args: &UndocApiArguments,
) -> anyhow::Result<()> {
    let (client, needs_start) = match client {
        Some(client) => (client, true),
        None => match state.get_undoc_client().await {
            Some(client) => (client, false),
            None => return Ok(()),
        },
    };

    log::info!("Querying undocumented API for device + room list");
    let acct = client.login_account_cached().await?;
    let info = client.get_device_list(&acct.token).await?;
    let mut group_by_id = HashMap::new();
    for group in info.groups {
        group_by_id.insert(group.group_id, group.group_name);
    }
    for entry in info.devices {
        let mut device = state.device_mut(&entry.sku, &entry.device).await;
        let room_name = group_by_id.get(&entry.group_id).map(|name| name.as_str());
        device.set_undoc_device_info(entry, room_name);
    }

    if needs_start {
        start_iot_client(args, state.clone(), Some(acct)).await?;
    }
    Ok(())
}

const ISSUE_76_EXPLANATION: &str = "Startup cannot automatically continue because entity names\n\
    could become inconsistent especially across frequent similar\n\
    intermittent issues if/as they occur on an ongoing basis.\n\
    Please see https://github.com/wez/govee2mqtt/issues/76\n\
    A workaround is to remove the Govee API credentials from your\n\
    configuration, which will cause this govee2mqtt to use only\n\
    the LAN API. Two consequences of that will be loss of control\n\
    over devices that do not support the LAN API, and also devices\n\
    changing entity ID to less descriptive names due to lack of\n\
    metadata availability via the LAN API.";

impl ServeCommand {
    pub async fn run(&self, args: &crate::Args) -> anyhow::Result<()> {
        log::info!("Starting service. version {}", govee_version());

        let timeout_secs = match self.availability_timeout {
            Some(secs) => Some(secs),
            None => govee_api::opt_env_var("GOVEE2MQTT_AVAILABILITY_TIMEOUT")?,
        }
        .unwrap_or(DEFAULT_AVAILABILITY_TIMEOUT_SECS);
        if timeout_secs < 1 {
            anyhow::bail!("availability-timeout must be at least 1 second, got {timeout_secs}");
        }
        // OnceCell: run is called once per process, so set can't already be filled.
        AVAILABILITY_TIMEOUT
            .set(chrono::Duration::seconds(timeout_secs))
            .ok();
        log::info!("Device availability timeout: {timeout_secs}s");

        let state = Arc::new(crate::service::state::State::new());

        // First, use the HTTP APIs to determine the list of devices and
        // their names.

        if let Ok(client) = args.api_args.api_client() {
            if let Err(err) =
                enumerate_devices_via_platform_api(state.clone(), Some(client.clone())).await
            {
                anyhow::bail!(
                    "Error during initial platform API discovery: {err:#}\n{ISSUE_76_EXPLANATION}"
                );
            }

            // only record the client after we've completed the
            // initial platform disco attempt
            state.set_platform_client(client).await;

            if let Ok(api_key) = args.api_args.api_key() {
                if let Err(err) = start_platform_iot(api_key, state.clone()).await {
                    log::warn!("Platform MQTT event subscribe failed to start: {err:#}");
                }
            }

            // spawn periodic discovery task
            let state = state.clone();
            tokio::spawn(async move {
                loop {
                    sleep(Duration::from_secs(600)).await;
                    if let Err(err) = enumerate_devices_via_platform_api(state.clone(), None).await
                    {
                        log::error!("Error during periodic platform API discovery: {err:#}");
                    }
                }
            });
        }
        if let Ok(client) = args.undoc_args.api_client() {
            if let Err(err) = enumerate_devices_via_undo_api(
                state.clone(),
                Some(client.clone()),
                &args.undoc_args,
            )
            .await
            {
                anyhow::bail!(
                    "Error during initial undoc API discovery: {err:#}\n{ISSUE_76_EXPLANATION}"
                );
            }

            // only record the client after we've completed the
            // initial undoc disco attempt
            state.set_undoc_client(client).await;

            // spawn periodic discovery task
            let state = state.clone();
            let args = args.undoc_args.clone();
            tokio::spawn(async move {
                loop {
                    sleep(Duration::from_secs(600)).await;
                    if let Err(err) =
                        enumerate_devices_via_undo_api(state.clone(), None, &args).await
                    {
                        log::error!("Error during periodic undoc API discovery: {err:#}");
                    }
                }
            });
        }

        // Now start LAN discovery

        let options = args.lan_disco_args.to_disco_options()?;
        let lan_enabled = !options.is_empty();
        if lan_enabled {
            log::info!("Starting LAN discovery");
            let state = state.clone();
            let (client, mut scan) = LanClient::new(options).await?;

            state.set_lan_client(client.clone()).await;

            tokio::spawn(async move {
                while let Some(lan_device) = scan.recv().await {
                    log::trace!("LAN disco: {lan_device:?}");
                    state
                        .device_mut(&lan_device.sku, &lan_device.device)
                        .await
                        .set_lan_device(lan_device.clone());

                    let state = state.clone();
                    let client = client.clone();
                    tokio::spawn(async move {
                        if let Ok(status) = client.query_status(&lan_device).await {
                            state
                                .device_mut(&lan_device.sku, &lan_device.device)
                                .await
                                .set_lan_device_status(status);

                            log::trace!("LAN disco: update and notify {}", lan_device.device);
                            state.notify_of_state_change(&lan_device.device).await.ok();
                        }
                    });
                }
            });
        }

        // Start direct BLE control if requested and a Bluetooth adapter exists.
        // When no adapter is found start_ble_client returns None and the BLE
        // client is never set, so the transport cascade skips BLE.
        if self.enable_ble {
            match crate::service::ble::start_ble_client().await {
                Ok(Some(client)) => state.set_ble_client(client).await,
                Ok(None) => {}
                Err(err) => log::warn!("Could not start direct BLE control: {err:#}"),
            }
        }

        // Log the resolved device set once, off the startup critical path. When
        // LAN discovery is enabled we wait for it to settle first, so the
        // "didn't respond to LAN probing" warnings don't fire for devices that
        // simply hadn't been probed yet (query_status has a 10s timeout).
        // Startup does not block on this: hass registration and polling start
        // immediately below, and each device's state streams into hass via
        // notify_of_state_change as discovery and polling respond.
        {
            let state = state.clone();
            tokio::spawn(async move {
                if lan_enabled {
                    sleep(Duration::from_secs(10)).await;
                }
                log_device_summary(&state).await;
            });
        }

        // Start periodic status polling
        {
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(err) = periodic_state_poll(state).await {
                    log::error!("periodic_state_poll: {err:#}");
                }
            });
        }

        // start advertising on local mqtt
        spawn_hass_integration(state.clone(), &args.hass_args).await?;

        run_http_server(state.clone(), self.http_port)
            .await
            .with_context(|| format!("Starting HTTP service on port {}", self.http_port))
    }
}
