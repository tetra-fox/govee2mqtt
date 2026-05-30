use crate::hass_mqtt::base::{Availability, Device as HassDevice};
use crate::hass_mqtt::topic::Topics;
use crate::service::ble::BleClient;
use crate::service::control::controller_for;
use crate::service::coordinator::Coordinator;
use crate::service::device::{Device, DeviceItem};
use crate::service::hass::{HassClient, topic_safe_id};
use crate::service::info::ServiceInfo;
use crate::service::iot::IotClient;
use anyhow::Context;
use chrono::{DateTime, Utc};
use govee_api::lan_api::Client as LanClient;
use govee_api::platform_api::{
    DeviceCapability, DeviceType, GoveeApiClient, sort_and_dedup_scenes,
};
use govee_api::temperature::{TemperatureScale, TemperatureValue};
use govee_api::undoc_api::GoveeUndocumentedApi;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Arc;
use tokio::sync::{MappedMutexGuard, Mutex, MutexGuard, Notify, Semaphore, broadcast};

/// Events fanned out to subscribers of state.subscribe(). Currently the only
/// internal consumer is the http /ws handler; mqtt publication stays in its
/// own path through notify_of_state_change.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StateEvent {
    /// Full current device list. Sent on ws connect and on broadcast lag so
    /// a client that fell behind can resync without reconnecting.
    Snapshot { devices: Vec<DeviceItem> },
    /// One device's state changed. Subscribers patch their local view by id.
    DeviceUpdated { device: DeviceItem },
    /// A control command finished (success or failure). UI shows these in
    /// the per-device command history without needing to poll.
    CommandLogged {
        device_id: String,
        entry: CommandLog,
    },
    /// A wire frame went out or arrived. Used by the frames inspector. Covers
    /// both directions: outbound BLE commands/reads/keepalive and IoT publishes,
    /// inbound BLE notifications and IoT status messages.
    Frame {
        device_id: String,
        direction: FrameDirection,
        transport: FrameTransport,
        ts: DateTime<Utc>,
        /// Hex string for BLE (e.g. "33 01 ff ..."), JSON string for IoT.
        payload: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotation: Option<govee_api::ble::FrameAnnotation>,
    },
}

#[derive(Serialize, Clone, Copy, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FrameDirection {
    Out,
    In,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FrameTransport {
    Ble,
    Iot,
    Lan,
}

/// Transports the cascade can pick to service a control command. Returned
/// from each verb so the wrapper can record which transport actually carried
/// the command for the debug surface; serialized as snake_case strings on
/// the wire.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// Govee LAN API over the device's own ip.
    Lan,
    /// Direct BLE, when the host has an adapter and the device exposes one.
    Ble,
    /// Owned-device AWS IoT MQTT.
    Iot,
    /// Govee platform REST API.
    Platform,
}

/// One control command's outcome as recorded for the debug surface. Captures
/// the verb, when it ran, how long it took, and which transport handled it
/// (or the error if all transports refused). Stored in a per-device ring on
/// State and pushed live over the ws.
///
/// `verb` + `args` are structured rather than a pre-formatted string so
/// consumers decide how to render. The daemon doesn't pick a display format.
#[derive(Serialize, Clone, Debug)]
pub struct CommandLog {
    /// Name of the cascade verb, e.g. "power_on" or "set_brightness".
    pub verb: String,
    /// Positional args as JSON values. Numbers stay numeric, booleans stay
    /// boolean, strings stay strings. Empty when the verb takes no args.
    pub args: Vec<JsonValue>,
    pub started: DateTime<Utc>,
    pub finished: DateTime<Utc>,
    pub outcome: CommandOutcome,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandOutcome {
    /// The cascade found a transport that accepted the command.
    Ok { transport: Transport },
    /// All transports refused; the final error message from the daemon.
    Err { message: String },
}

/// Per-device command-history ring. The cap matches what the UI shows; bumping
/// it costs proportional memory per active device, so keep it modest.
pub const COMMAND_HISTORY_CAP: usize = 30;

/// Global ring of recent wire frames. Sized to match the UI's FRAME_TAIL_CAP so
/// the two views align on backfill. Each entry is one frame from `notify_frame`,
/// regardless of device — frames carry their own device_id. Average payload is
/// well under 1 KB (BLE frames are 60 chars of hex, IoT envelopes are a few
/// hundred bytes), so 1000 entries is roughly 0.5-1 MB resident.
pub const FRAME_HISTORY_CAP: usize = 1000;

/// One entry of the frame ring. Same wire shape as the `Frame` ws event minus
/// the `type` discriminant; the UI deserializes them into its `Frame` type.
#[derive(Serialize, Clone, Debug)]
pub struct RecentFrame {
    pub device_id: String,
    pub direction: FrameDirection,
    pub transport: FrameTransport,
    pub ts: DateTime<Utc>,
    pub payload: String,
    /// Per-byte decode of a BLE frame for the inspector, from the device's SKU.
    /// None for IoT frames (the ui decodes their JSON) and for BLE frames whose
    /// SKU we can't resolve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation: Option<govee_api::ble::FrameAnnotation>,
}

/// Parse a space-separated hex frame (the BLE inspector wire format produced by
/// `hex_pretty`) back into bytes. None if any token isn't valid hex.
fn parse_hex_frame(hex: &str) -> Option<Vec<u8>> {
    hex.split_whitespace()
        .map(|tok| u8::from_str_radix(tok, 16).ok())
        .collect()
}

pub struct State {
    devices_by_id: Mutex<HashMap<String, Device>>,
    semaphore_by_id: Mutex<HashMap<String, Arc<Semaphore>>>,
    lan_client: Mutex<Option<LanClient>>,
    platform_client: Mutex<Option<GoveeApiClient>>,
    undoc_client: Mutex<Option<GoveeUndocumentedApi>>,
    iot_client: Mutex<Option<IotClient>>,
    /// Present only when direct BLE is enabled and a Bluetooth adapter exists;
    /// the transport cascade skips BLE when this is None.
    ble_client: Mutex<Option<BleClient>>,
    hass_client: Mutex<Option<HassClient>>,
    hass_discovery_prefix: Mutex<String>,
    base_topic: Mutex<String>,
    temperature_scale: Mutex<TemperatureScale>,
    /// The device-discovery components published during the most recent
    /// registration, keyed by device config topic. Each value holds the
    /// device/origin/availability blocks plus a per-component map of
    /// {platform, config_json}. Held so the next registration can: clear a
    /// device topic we no longer produce (empty retained payload removes the
    /// whole device), and tombstone a single component that a still-present
    /// device no longer produces (republished as `{"p": platform}`, home
    /// assistant's signal to drop just that component). The same map is
    /// surfaced by the debug endpoint, which is why it keeps the full
    /// per-component config json rather than just the platform string.
    published_components: Mutex<PublishedComponents>,
    /// Wall-clock time of the last completed register_with_hass pass and
    /// whether that pass had a complete enumeration. Surfaced by the debug
    /// endpoint so a debugger can tell whether the published configs reflect
    /// the current device set or an older partial pass.
    last_registration: Mutex<Option<RegistrationStatus>>,
    /// Serializes registration so two overlapping registrations (eg: an HA
    /// birth message arriving during a reconnect re-register) can't both diff
    /// against a half-rebuilt topic set.
    registration_lock: Mutex<()>,
    /// Signalled once the IoT subscriber has connected and subscribed to the
    /// account topic. Status replies arrive on that topic, so a status request
    /// published before the subscription is live gets no reply; the first poll
    /// waits on this so it doesn't fire into the void. notify_one stores a
    /// permit, so a wait that starts after the signal still completes.
    iot_ready: Notify,
    /// Fan-out for ui websocket subscribers. send is non-blocking and returns
    /// Err when no receivers exist (the normal case when no ui is open), so
    /// the publish path drops the result. Capacity 256 absorbs slow clients;
    /// a receiver that falls further behind gets a Lagged error and is
    /// expected to resync from a fresh Snapshot.
    events: broadcast::Sender<StateEvent>,
    /// Per-device ring of the last COMMAND_HISTORY_CAP control commands run
    /// through `device_*` wrappers. Populated by the wrappers, exposed via the
    /// debug endpoint and the ws CommandLogged event. Keyed by device id.
    command_history: Mutex<HashMap<String, VecDeque<CommandLog>>>,
    /// Global ring of the last FRAME_HISTORY_CAP frames seen on any device.
    /// Populated by `notify_frame` alongside the ws fan-out so a ui that opens
    /// after some traffic can backfill the inspector without losing context.
    frame_history: Mutex<VecDeque<RecentFrame>>,
    /// Snapshot of the daemon's configuration captured once at serve startup;
    /// surfaced by the info endpoint. None until set_service_info runs, which
    /// only happens in the serve subcommand.
    service_info: Mutex<Option<ServiceInfo>>,
}

impl Default for State {
    fn default() -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            devices_by_id: Default::default(),
            semaphore_by_id: Default::default(),
            lan_client: Default::default(),
            platform_client: Default::default(),
            undoc_client: Default::default(),
            iot_client: Default::default(),
            ble_client: Default::default(),
            hass_client: Default::default(),
            hass_discovery_prefix: Default::default(),
            base_topic: Default::default(),
            temperature_scale: Default::default(),
            published_components: Default::default(),
            last_registration: Default::default(),
            registration_lock: Default::default(),
            iot_ready: Default::default(),
            events,
            command_history: Default::default(),
            frame_history: Default::default(),
            service_info: Default::default(),
        }
    }
}

pub type StateHandle = Arc<State>;

/// One device's discovery payload, retained between registration passes so the
/// next pass can diff against it and so the debug endpoint can show what HA
/// actually saw. Mirrors [`crate::hass_mqtt::base::DeviceDiscovery`] with two
/// shape changes: components are broken back out keyed by unique id (so the
/// UI can show each on its own), and `origin` is omitted because it's
/// identical across every device by construction (hoisted to the bundle level
/// in the debug response).
#[derive(Serialize, Clone, Debug)]
pub struct PublishedDevice {
    pub device: HassDevice,
    pub availability: Vec<Availability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_mode: Option<&'static str>,
    pub components: BTreeMap<String, PublishedComponent>,
}

/// One entity's contribution to a device-discovery payload, as we published it.
/// `config` is the per-component JSON with the hoisted device/origin/availability
/// blocks already stripped (mirrors what the device-discovery payload carries).
#[derive(Serialize, Clone, Debug)]
pub struct PublishedComponent {
    pub platform: String,
    pub config: JsonValue,
}

/// Device config topic -> the discovery payload we published for that device
/// in the most recent registration pass.
pub type PublishedComponents = HashMap<String, PublishedDevice>;

/// Wall-clock timestamp of the most recent COMPLETE register_with_hass pass,
/// surfaced by the debug endpoint. Partial passes don't update this — the
/// retained configs they left in place came from the previous complete pass,
/// and that pass's timestamp is what describes their freshness.
#[derive(Serialize, Clone, Copy, Debug)]
pub struct RegistrationStatus {
    pub at: DateTime<Utc>,
}

impl State {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to state-change events. Each receiver gets every event sent
    /// after it subscribed; lag past the 256-slot ring yields RecvError::Lagged
    /// and the receiver should re-snapshot rather than reconnect.
    pub fn subscribe(&self) -> broadcast::Receiver<StateEvent> {
        self.events.subscribe()
    }

    /// Cloneable sender so subsystems that don't hold a StateHandle (the IoT
    /// client, in particular) can emit events without circular Arcs.
    pub fn event_sender(&self) -> broadcast::Sender<StateEvent> {
        self.events.clone()
    }

    /// Record a frame and broadcast it to ws subscribers. Always pushes into
    /// the global ring (so a refreshing/late-connecting ui can backfill via
    /// `/api/recent`); only broadcasts when there's a live subscriber to send
    /// to. The ring entry and the broadcast carry identical wire data.
    pub fn notify_frame(
        &self,
        device_id: &str,
        sku: &str,
        direction: FrameDirection,
        transport: FrameTransport,
        payload: String,
    ) {
        let ts = Utc::now();
        // Annotate BLE frames per-byte from the device's SKU, so the inspector
        // names bytes from the same codec that decodes them. IoT frames are JSON
        // and the ui decodes those itself.
        let annotation = match transport {
            FrameTransport::Ble => {
                parse_hex_frame(&payload).map(|bytes| govee_api::ble::annotate_frame(sku, &bytes))
            }
            // IoT and LAN frames are JSON; the ui decodes those itself.
            FrameTransport::Iot | FrameTransport::Lan => None,
        };
        let entry = RecentFrame {
            device_id: device_id.to_string(),
            direction,
            transport,
            ts,
            payload,
            annotation,
        };
        // Drop into the ring synchronously so a refresh that happens within
        // milliseconds of a frame still sees it. try_lock keeps the wire-send
        // path lock-free in the unlikely case of contention; missing one
        // ring entry is preferable to blocking instrumentation.
        if let Ok(mut ring) = self.frame_history.try_lock() {
            if ring.len() >= FRAME_HISTORY_CAP {
                ring.pop_front();
            }
            ring.push_back(entry.clone());
        }
        if self.events.receiver_count() == 0 {
            return;
        }
        let _ = self.events.send(StateEvent::Frame {
            device_id: entry.device_id,
            direction: entry.direction,
            transport: entry.transport,
            ts: entry.ts,
            payload: entry.payload,
            annotation: entry.annotation,
        });
    }

    /// Record a LAN UDP frame from the lan_api capture hook. Resolves the device
    /// by its LAN ip and records it as a Lan frame. An ip we don't have a device
    /// for (e.g. a scan response from a device not yet registered) is dropped,
    /// since the inspector keys frames by device.
    pub async fn notify_lan_frame(&self, ip: IpAddr, direction: FrameDirection, json: String) {
        let resolved = {
            let devices = self.devices_by_id.lock().await;
            devices.values().find_map(|d| match d.lan_device.as_ref() {
                Some(lan) if lan.ip == ip => Some((d.id.clone(), d.sku.clone())),
                _ => None,
            })
        };
        let Some((device_id, sku)) = resolved else {
            return;
        };
        self.notify_frame(&device_id, &sku, direction, FrameTransport::Lan, json);
    }

    /// Read-only copy of the recent-frames ring (oldest first).
    pub async fn get_recent_frames(&self) -> Vec<RecentFrame> {
        self.frame_history.lock().await.iter().cloned().collect()
    }

    /// All known per-device command histories. Returned as a map so the ui
    /// can hydrate every device at once instead of round-tripping per id.
    pub async fn get_all_command_histories(&self) -> HashMap<String, Vec<CommandLog>> {
        self.command_history
            .lock()
            .await
            .iter()
            .map(|(id, ring)| (id.clone(), ring.iter().cloned().collect()))
            .collect()
    }

    pub async fn set_temperature_scale(&self, scale: TemperatureScale) {
        *self.temperature_scale.lock().await = scale;
    }

    pub async fn get_temperature_scale(&self) -> TemperatureScale {
        *self.temperature_scale.lock().await
    }

    pub async fn set_hass_disco_prefix(&self, prefix: String) {
        *self.hass_discovery_prefix.lock().await = prefix;
    }

    pub async fn get_hass_disco_prefix(&self) -> String {
        self.hass_discovery_prefix.lock().await.to_string()
    }

    /// Hold the registration lock for the duration of a registration pass.
    pub async fn lock_registration(&self) -> MutexGuard<'_, ()> {
        self.registration_lock.lock().await
    }

    /// Take the components published in the previous registration pass, leaving
    /// the stored map empty. The pass diffs the live components against this to
    /// find devices and components that went away, then stores the new map via
    /// `set_published_components`. Caller must hold the registration lock so the
    /// take and the later set aren't interleaved with another pass.
    pub async fn take_published_components(&self) -> PublishedComponents {
        std::mem::take(&mut *self.published_components.lock().await)
    }

    /// Store the components published in the current registration pass, for the
    /// next pass to diff against.
    pub async fn set_published_components(&self, components: PublishedComponents) {
        *self.published_components.lock().await = components;
    }

    /// Read-only snapshot of the currently-published HA components, for the
    /// /api/debug/hass endpoint. Clones so the caller doesn't hold the lock.
    pub async fn get_published_components(&self) -> PublishedComponents {
        self.published_components.lock().await.clone()
    }

    /// Stamp the most recent registration pass: the wall-clock time and
    /// whether the enumeration was complete (vs. partial-and-left-alone).
    pub async fn set_last_registration(&self, status: RegistrationStatus) {
        *self.last_registration.lock().await = Some(status);
    }

    pub async fn get_last_registration(&self) -> Option<RegistrationStatus> {
        *self.last_registration.lock().await
    }

    /// Push one command-history entry onto a device's ring and broadcast it
    /// to ws subscribers. The ring evicts the oldest entry once full.
    pub async fn record_command_log(self: &Arc<Self>, device_id: &str, entry: CommandLog) {
        {
            let mut histories = self.command_history.lock().await;
            let ring = histories.entry(device_id.to_string()).or_default();
            if ring.len() >= COMMAND_HISTORY_CAP {
                ring.pop_front();
            }
            ring.push_back(entry.clone());
        }
        if self.events.receiver_count() > 0 {
            let _ = self.events.send(StateEvent::CommandLogged {
                device_id: device_id.to_string(),
                entry,
            });
        }
    }

    pub async fn set_service_info(&self, info: ServiceInfo) {
        *self.service_info.lock().await = Some(info);
    }

    pub async fn get_service_info(&self) -> Option<ServiceInfo> {
        self.service_info.lock().await.clone()
    }

    /// Read-only copy of a device's command history (newest last).
    pub async fn get_command_history(&self, device_id: &str) -> Vec<CommandLog> {
        self.command_history
            .lock()
            .await
            .get(device_id)
            .map(|ring| ring.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn set_base_topic(&self, base_topic: String) {
        *self.base_topic.lock().await = base_topic;
    }

    pub async fn get_base_topic(&self) -> String {
        self.base_topic.lock().await.clone()
    }

    /// The topic/unique-id builder, seeded from the configured base topic.
    pub async fn topics(&self) -> Topics {
        Topics::new(self.base_topic.lock().await.clone())
    }

    /// Returns a mutable version of the specified device, creating
    /// an entry for it if necessary.
    pub async fn device_mut(&self, sku: &str, id: &str) -> MappedMutexGuard<'_, Device> {
        let devices = self.devices_by_id.lock().await;
        MutexGuard::map(devices, |devices| {
            devices
                .entry(id.to_string())
                .or_insert_with(|| Device::new(sku, id))
        })
    }

    pub async fn devices(&self) -> Vec<Device> {
        self.devices_by_id.lock().await.values().cloned().collect()
    }

    /// Returns an immutable copy of the specified Device
    pub async fn device_by_id(&self, id: &str) -> Option<Device> {
        let devices = self.devices_by_id.lock().await;
        devices.get(id).cloned()
    }

    async fn semaphore_for_device(&self, device: &Device) -> Arc<Semaphore> {
        self.semaphore_by_id
            .lock()
            .await
            .entry(device.id.clone())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    }

    pub async fn resolve_device_read_only(self: &Arc<Self>, label: &str) -> anyhow::Result<Device> {
        self.resolve_device(label)
            .await
            .ok_or_else(|| anyhow::anyhow!("device '{label}' not found"))
    }

    /// Resolve a device based on its label.
    /// Assuming the device is found, returns a Coordinator, which is a
    /// struct that ensures that only one task at a time can be processing
    /// control requests for a device.
    /// This method will not return until the calling task is permitted
    /// to proceed with its control attempt.
    pub async fn resolve_device_for_control(
        self: &Arc<Self>,
        label: &str,
    ) -> anyhow::Result<Coordinator> {
        let device = self
            .resolve_device(label)
            .await
            .ok_or_else(|| anyhow::anyhow!("device '{label}' not found"))?;
        let semaphore = self.semaphore_for_device(&device).await;
        let permit = semaphore.acquire_owned().await?;
        let (tx, rx) = tokio::sync::oneshot::channel();

        // Schedule a task that will poll the device a short
        // time after the Coordinator is dropped, to reconcile
        // any changed state
        let state = self.clone();
        let device_id = device.id.to_string();
        tokio::spawn(async move {
            let _ = rx.await;
            state.poll_after_control(device_id).await
        });

        Ok(Coordinator::new(device, permit, tx))
    }

    /// Resolve a device using its name, computed name, id or label,
    /// ignoring case.
    pub async fn resolve_device(&self, label: &str) -> Option<Device> {
        let devices = self.devices_by_id.lock().await;

        // Try by id first
        if let Some(device) = devices.get(label) {
            return Some(device.clone());
        }

        for d in devices.values() {
            if d.name().eq_ignore_ascii_case(label)
                || d.id.eq_ignore_ascii_case(label)
                || topic_safe_id(d).eq_ignore_ascii_case(label)
                || d.ip_addr()
                    .map(|ip| ip.to_string().eq_ignore_ascii_case(label))
                    .unwrap_or(false)
                || d.computed_name().eq_ignore_ascii_case(label)
            {
                return Some(d.clone());
            }
        }

        None
    }

    pub async fn set_hass_client(&self, client: HassClient) {
        self.hass_client.lock().await.replace(client);
    }

    pub async fn get_hass_client(&self) -> Option<HassClient> {
        self.hass_client.lock().await.clone()
    }

    pub async fn set_iot_client(&self, client: IotClient) {
        self.iot_client.lock().await.replace(client);
    }

    pub async fn get_iot_client(&self) -> Option<IotClient> {
        self.iot_client.lock().await.clone()
    }

    pub async fn set_ble_client(&self, client: BleClient) {
        self.ble_client.lock().await.replace(client);
    }

    pub async fn get_ble_client(&self) -> Option<BleClient> {
        self.ble_client.lock().await.clone()
    }

    /// Mark the IoT client connected and subscribed; the IoT subscriber calls
    /// this on ConnAck after it subscribes to the account topic.
    pub fn signal_iot_ready(&self) {
        self.iot_ready.notify_one();
    }

    /// Wait until the IoT client has connected and subscribed, so a status
    /// request will actually get a reply.
    pub async fn wait_for_iot_ready(&self) {
        self.iot_ready.notified().await;
    }

    pub async fn set_lan_client(&self, client: LanClient) {
        self.lan_client.lock().await.replace(client);
    }

    pub async fn get_lan_client(&self) -> Option<LanClient> {
        self.lan_client.lock().await.clone()
    }

    pub async fn set_platform_client(&self, client: GoveeApiClient) {
        self.platform_client.lock().await.replace(client);
    }

    pub async fn get_platform_client(&self) -> Option<GoveeApiClient> {
        self.platform_client.lock().await.clone()
    }

    pub async fn set_undoc_client(&self, client: GoveeUndocumentedApi) {
        self.undoc_client.lock().await.replace(client);
    }

    #[allow(dead_code)]
    pub async fn get_undoc_client(&self) -> Option<GoveeUndocumentedApi> {
        self.undoc_client.lock().await.clone()
    }

    pub async fn poll_iot_api(self: &Arc<Self>, device: &Device) -> anyhow::Result<bool> {
        if let Some(iot) = self.get_iot_client().await
            && let Some(info) = device.undoc_device_info.clone()
            && iot.is_device_compatible(&info.entry)
        {
            let device_state = device.device_state();
            log::debug!("requesting update via IoT MQTT {device} {device_state:?}");
            match iot
                .request_status_update(&info.entry)
                .await
                .context("iot.request_status_update")
            {
                Err(err) => {
                    log::error!("Failed: {err:#}");
                }
                Ok(()) => {
                    // The response will come in async via the mqtt loop in iot.rs
                    // However, if the device is offline, nothing will change our state.
                    // Let's explicitly mark the device as having been polled so that
                    // we don't keep sending a request every minute.
                    self.device_mut(&device.sku, &device.id)
                        .await
                        .set_last_polled();

                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub async fn poll_platform_api(self: &Arc<Self>, device: &Device) -> anyhow::Result<bool> {
        if let Some(client) = self.get_platform_client().await {
            if let DeviceType::Other(other) = &device.device_type() {
                // Cannot poll an unknown device
                // <https://github.com/wez/govee2mqtt/issues/391>
                // <https://github.com/wez/govee2mqtt/issues/501>
                // <https://github.com/wez/govee2mqtt/issues/394>
                log::trace!("device {device} cannot be polled because it has type Other: {other}");
                return Ok(false);
            }

            let device_state = device.device_state();
            log::debug!("requesting update via Platform API {device} {device_state:?}");
            if let Some(info) = &device.http_device_info {
                let http_state = client
                    .get_device_state(info)
                    .await
                    .context("get_device_state")?;
                log::trace!("updated state for {device}");

                {
                    let mut device = self.device_mut(&device.sku, &device.id).await;
                    device.set_http_device_state(http_state);
                    device.set_last_polled();
                }
                self.notify_of_state_change(&device.id)
                    .await
                    .context("state.notify_of_state_change")?;
                return Ok(true);
            }
        } else {
            log::trace!(
                "device {device} wanted a status update, but there is no platform client available"
            );
        }
        Ok(false)
    }

    pub async fn device_control<V: Into<JsonValue>>(
        self: &Arc<Self>,
        device: &Device,
        capability: &DeviceCapability,
        value: V,
    ) -> anyhow::Result<()> {
        let value: JsonValue = value.into();
        self.run_command(
            &device.id,
            "device_control",
            vec![json!(capability.instance), value.clone()],
            async {
                crate::service::transport::device_control(self, device, capability, value.clone())
                    .await
            },
        )
        .await
    }

    /// Try to control `instance` as a ptReal frame; returns the transport that
    /// carried it, or None if it isn't a frame-encoded instance for this SKU
    /// (caller falls back).
    pub async fn try_frame_capability(
        self: &Arc<Self>,
        device: &Device,
        instance: &str,
        value: &JsonValue,
    ) -> anyhow::Result<Option<Transport>> {
        crate::service::transport::try_frame_capability(self, device, instance, value).await
    }

    pub async fn device_light_power_on(
        self: &Arc<Self>,
        device: &Device,
        on: bool,
    ) -> anyhow::Result<()> {
        self.run_command(&device.id, "light_power_on", vec![json!(on)], async {
            controller_for(device)
                .light_power_on(self, device, on)
                .await
        })
        .await
    }

    pub async fn device_power_on(
        self: &Arc<Self>,
        device: &Device,
        on: bool,
    ) -> anyhow::Result<()> {
        self.run_command(&device.id, "power_on", vec![json!(on)], async {
            controller_for(device).power_on(self, device, on).await
        })
        .await
    }

    pub async fn device_set_brightness(
        self: &Arc<Self>,
        device: &Device,
        percent: u8,
    ) -> anyhow::Result<()> {
        self.run_command(&device.id, "set_brightness", vec![json!(percent)], async {
            controller_for(device)
                .set_brightness(self, device, percent)
                .await
        })
        .await
    }

    pub async fn device_set_color_temperature(
        self: &Arc<Self>,
        device: &Device,
        kelvin: u32,
    ) -> anyhow::Result<()> {
        self.run_command(
            &device.id,
            "set_color_temperature",
            vec![json!(kelvin)],
            async {
                controller_for(device)
                    .set_color_temperature(self, device, kelvin)
                    .await
            },
        )
        .await
    }

    pub async fn device_set_color_rgb(
        self: &Arc<Self>,
        device: &Device,
        r: u8,
        g: u8,
        b: u8,
    ) -> anyhow::Result<()> {
        self.run_command(
            &device.id,
            "set_color_rgb",
            vec![json!(r), json!(g), json!(b)],
            async {
                controller_for(device)
                    .set_color_rgb(self, device, r, g, b)
                    .await
            },
        )
        .await
    }

    /// THE chokepoint for every device verb that mutates state. The inner
    /// future runs the actual cascade and returns which transport carried the
    /// command; this wrapper records when it started, when it finished, the
    /// outcome, and pushes a CommandLog onto the device's history ring (which
    /// also fans out a `command_logged` ws event). Adding a new public
    /// `device_*` verb without routing it through here silently breaks the
    /// debug surface, which is exactly why every existing verb is a one-call
    /// wrapper around `run_command`.
    ///
    /// `verb` is the cascade verb name; `args` are the call arguments as JSON
    /// values. Consumers (UI, CLI) format these for display, the daemon
    /// doesn't pick a render shape.
    async fn run_command(
        self: &Arc<Self>,
        device_id: &str,
        verb: &str,
        args: Vec<JsonValue>,
        inner: impl Future<Output = anyhow::Result<Transport>>,
    ) -> anyhow::Result<()> {
        let started = Utc::now();
        let result = inner.await;
        let outcome = match &result {
            Ok(transport) => CommandOutcome::Ok {
                transport: *transport,
            },
            Err(e) => CommandOutcome::Err {
                message: format!("{e:#}"),
            },
        };
        self.record_command_log(
            device_id,
            CommandLog {
                verb: verb.to_string(),
                args,
                started,
                finished: Utc::now(),
                outcome,
            },
        )
        .await;
        result.map(|_| ())
    }

    pub async fn humidifier_set_parameter(
        self: &Arc<Self>,
        device: &Device,
        work_mode: i64,
        value: i64,
    ) -> anyhow::Result<()> {
        self.run_command(
            &device.id,
            "humidifier_set_parameter",
            vec![json!(work_mode), json!(value)],
            async {
                crate::service::transport::humidifier_set_parameter(self, device, work_mode, value)
                    .await
            },
        )
        .await
    }

    /// Set the speed of a fan via its workMode capability. `work_mode` is the
    /// "FanSpeed" mode number (per ParsedWorkMode); `speed` is the integer
    /// speed level in `[1, fan_speed_max]`.
    pub async fn fan_set_speed(
        self: &Arc<Self>,
        device: &Device,
        work_mode: i64,
        speed: i64,
    ) -> anyhow::Result<()> {
        self.run_command(
            &device.id,
            "fan_set_speed",
            vec![json!(work_mode), json!(speed)],
            async {
                crate::service::transport::fan_set_speed(self, device, work_mode, speed).await
            },
        )
        .await
    }

    /// Switch a single outlet of a multi-outlet socket (eg: H5082).
    /// <https://github.com/wez/govee2mqtt/issues/65>
    pub async fn device_set_socket_outlet(
        self: &Arc<Self>,
        device: &Device,
        index: u8,
        on: bool,
    ) -> anyhow::Result<()> {
        self.run_command(
            &device.id,
            "set_socket_outlet",
            vec![json!(index), json!(on)],
            async { crate::service::transport::socket_turn(self, device, index, on).await },
        )
        .await
    }

    pub async fn poll_after_control(self: &Arc<Self>, id: String) {
        crate::service::transport::poll_after_control(self, id).await
    }

    pub async fn device_list_scenes(&self, device: &Device) -> anyhow::Result<Vec<String>> {
        // TODO: some plumbing to maintain offline scene controls for preferred-LAN control
        if let Some(client) = self.get_platform_client().await
            && let Some(info) = &device.http_device_info
        {
            return Ok(sort_and_dedup_scenes(client.list_scene_names(info).await?));
        }

        if let Ok(categories) = GoveeUndocumentedApi::get_scenes_for_device(&device.sku).await {
            let mut names = vec![];
            for cat in categories {
                for scene in cat.scenes {
                    for effect in scene.light_effects {
                        if effect.scene_code != 0 {
                            names.push(scene.scene_name);
                            break;
                        }
                    }
                }
            }
            return Ok(sort_and_dedup_scenes(names));
        }

        log::trace!("Platform API unavailable: Don't know how to list scenes for {device}");

        Ok(vec![])
    }

    pub async fn device_set_target_temperature(
        self: &Arc<Self>,
        device: &Device,
        instance_name: &str,
        target: TemperatureValue,
    ) -> anyhow::Result<()> {
        // TemperatureValue's Display is "21.5C" / "70.0F" so it carries the
        // scale glyph; encoded as a string in args since the temperature
        // type isn't directly serializable to a numeric JSON value here.
        self.run_command(
            &device.id,
            "set_target_temperature",
            vec![json!(instance_name), json!(target.to_string())],
            async {
                if let Some(client) = self.get_platform_client().await
                    && let Some(info) = &device.http_device_info
                {
                    log::debug!(
                        "Using Platform API to set {device} target temperature to {target}"
                    );
                    client
                        .set_target_temperature(info, instance_name, target, None)
                        .await?;
                    return Ok(Transport::Platform);
                }
                anyhow::bail!("Unable to set temperature for {device}");
            },
        )
        .await
    }

    pub async fn device_set_scene(
        self: &Arc<Self>,
        device: &Device,
        scene: &str,
    ) -> anyhow::Result<()> {
        self.run_command(&device.id, "set_scene", vec![json!(scene)], async {
            crate::service::transport::device_set_scene(self, device, scene).await
        })
        .await
    }

    // Take care not to call this while you hold a mutable device
    // reference, as that will deadlock!
    pub async fn notify_of_state_change(self: &Arc<Self>, device_id: &str) -> anyhow::Result<()> {
        let Some(canonical_device) = self.device_by_id(device_id).await else {
            anyhow::bail!("cannot find device {device_id}!?");
        };

        // skip the snapshot allocation when no ws client is connected. send
        // returns Err only when receiver_count is zero, but the snapshot work
        // happens before send, so the guard is what actually saves the clones.
        if self.events.receiver_count() > 0 {
            let _ = self.events.send(StateEvent::DeviceUpdated {
                device: DeviceItem::snapshot(&canonical_device),
            });
        }

        if let Some(hass) = self.get_hass_client().await {
            hass.advise_hass_of_light_state(&canonical_device, self)
                .await?;
        }

        Ok(())
    }
}
