//! Direct BLE (Bluetooth GATT) control of owned Govee devices, bypassing the
//! cloud. Mirrors the IoT client's role (a cloneable handle held in [`State`],
//! [`crate::service::state`]) but holds GATT connections instead of an MQTT
//! socket.
//!
//! Commands are the same 20-byte frames the BLE codec produces for the cloud
//! `op.command` path; here they are encrypted with the device's session
//! ([`govee_api::ble::encryption`]) and written to the GATT data characteristic.
//!
//! The client is only created when a Bluetooth adapter is present, so the
//! transport cascade skips BLE entirely on hosts without one (no per-command
//! connect/timeout cost). Connections are established lazily on first command and
//! reused; a dropped link is re-established (and re-handshaked) on next use.

use crate::service::state::{FrameDirection, FrameTransport, StateHandle};
use crate::service::transport::hex_pretty;
use anyhow::Context;
use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter,
    ValueNotification, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures_util::{Stream, StreamExt};
use govee_api::ble::encryption::{
    Session, Version, negotiate_version, random_iv_send, v1_build_confirm, v1_build_request,
    v1_is_confirm_ack, v1_parse_key_reply, v2_build_request, v2_parse_single_reply,
    v2_session_from_reply,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant, interval, sleep, timeout};
use uuid::Uuid;

/// Govee encrypted-control GATT UUIDs (Constants.java) share the base
/// `00010203-0405-0607-0809-0a0b0c0d____`; only the low 16 bits differ.
const fn govee_uuid(short: u16) -> Uuid {
    Uuid::from_u128(0x0001_0203_0405_0607_0809_0a0b_0c0d_0000 | short as u128)
}
const SERVICE_UUID: Uuid = govee_uuid(0x1910);
/// Command write channel; encrypted command frames go here.
const DATA_CHAR_UUID: Uuid = govee_uuid(0x2b11);
/// Notify channel: the device sends handshake replies and status here. Confirmed
/// from a captured session (the app enables notifications on 2b10, writes to 2b11).
const NOTIFY_CHAR_UUID: Uuid = govee_uuid(0x2b10);
/// BgcInfo read, used to negotiate the encryption version.
pub const BGC_CHAR_UUID: Uuid = govee_uuid(0x2b12);

const SCAN_SECS: u64 = 5;
/// Scan windows to try when finding a device; covers an intermittent advertiser.
const SCAN_ATTEMPTS: u32 = 6;
const REPLY_TIMEOUT_SECS: u64 = 6;
/// Idle period after which a pooled connection is dropped, freeing the device for
/// other BLE centrals. Bursts within this window reuse the warm session.
const IDLE_DISCONNECT_SECS: u64 = 30;
/// aa00 keepalive cadence. The app sends it every 3s to hold the link open; the
/// device echoes it back. Idle past this and the device drops the connection.
const HEARTBEAT_SECS: u64 = 3;
/// How often the status reader re-sends the read burst to refresh held state.
/// The link is held open the whole time, so this is just a re-poll cadence.
const READ_REFRESH_SECS: u64 = 30;
/// Delay before a status reader reconnects after the link drops.
const RECONNECT_DELAY_SECS: u64 = 5;
/// How often the reader manager re-scans the device set for new BLE devices to
/// start readers for.
const READER_RECONCILE_SECS: u64 = 60;

/// The aa00 keepalive frame: `aa` then 18 zero bytes, with the trailing xor
/// checksum (which is `aa`, the xor of the lead byte and the zero pad).
const HEARTBEAT_FRAME: [u8; 20] = {
    let mut frame = [0u8; 20];
    frame[0] = 0xaa;
    frame[19] = 0xaa;
    frame
};

#[derive(Clone)]
pub struct BleClient {
    inner: Arc<BleInner>,
}

struct BleInner {
    adapter: Adapter,
    /// Established links keyed by uppercased BLE MAC.
    links: Mutex<HashMap<String, Arc<DeviceLink>>>,
    /// Set once disconnect_all runs at shutdown, so the status readers stop
    /// re-establishing links we are tearing down.
    shutting_down: AtomicBool,
}

struct DeviceLink {
    peripheral: Peripheral,
    data_char: Characteristic,
    session: Mutex<Session>,
    /// Bumped on each command; the idle-release task only disconnects if it is
    /// unchanged after the idle window (i.e. no command arrived since).
    generation: AtomicU64,
    /// Set while a status reader is holding this link open. The idle-release task
    /// skips a held link, so the reader's keepalive isn't torn down by a stale
    /// idle timer armed by an earlier command.
    held: AtomicBool,
}

/// Result of a read-only [`BleClient::probe`].
pub struct ProbeReport {
    pub address: String,
    /// Advertisement manufacturer data (company id -> bytes); the Govee broadcast
    /// carries the supportEncryption flag here.
    pub manufacturer_data: Vec<(u16, Vec<u8>)>,
    /// Advertisement service data (service uuid -> bytes).
    pub service_data: Vec<(Uuid, Vec<u8>)>,
    /// (service uuid, characteristic uuid, properties) for every characteristic
    /// the device exposes, so an unexpected GATT layout is visible.
    pub characteristics: Vec<(Uuid, Uuid, CharPropFlags)>,
    /// BgcInfo bytes if the expected characteristic was found and read.
    pub bgc_info: Option<Vec<u8>>,
    pub version: Option<Version>,
}

/// Probe for a Bluetooth adapter and, if one exists, build a [`BleClient`].
/// Returns `None` when no adapter is present so the caller leaves BLE out of the
/// transport cascade.
pub async fn start_ble_client() -> anyhow::Result<Option<BleClient>> {
    let manager = Manager::new().await.context("creating BLE manager")?;
    let adapters = manager.adapters().await.context("listing BLE adapters")?;
    let Some(adapter) = adapters.into_iter().next() else {
        log::info!("No Bluetooth adapter found; direct BLE control disabled");
        return Ok(None);
    };
    let info = adapter
        .adapter_info()
        .await
        .unwrap_or_else(|_| "unknown adapter".to_string());
    log::info!("Direct BLE control enabled using {info}");
    Ok(Some(BleClient {
        inner: Arc::new(BleInner {
            adapter,
            links: Mutex::new(HashMap::new()),
            shutting_down: AtomicBool::new(false),
        }),
    }))
}

impl BleClient {
    /// Encrypt and write each command frame to the device over BLE, establishing
    /// (and handshaking) the connection on first use. The connection is kept warm
    /// and dropped after [`IDLE_DISCONNECT_SECS`] of no commands, so a burst reuses
    /// one session but the device is freed once you stop driving it.
    pub async fn send_frames(&self, ble_addr: &str, frames: &[Vec<u8>]) -> anyhow::Result<()> {
        let link = self.get_or_connect(ble_addr).await?;
        let result = write_to_link(&link, frames).await;
        self.arm_idle_disconnect(ble_addr, &link);
        result
    }

    /// Schedule a release of the pooled link after the idle window, unless another
    /// command bumps the generation first, or a status reader is holding the link.
    fn arm_idle_disconnect(&self, ble_addr: &str, link: &Arc<DeviceLink>) {
        let generation = link.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let client = self.clone();
        let key = ble_addr.to_uppercase();
        tokio::spawn(async move {
            sleep(Duration::from_secs(IDLE_DISCONNECT_SECS)).await;
            let link = {
                let mut links = client.inner.links.lock().await;
                match links.get(&key) {
                    Some(l)
                        if l.generation.load(Ordering::SeqCst) == generation
                            && !l.held.load(Ordering::SeqCst) =>
                    {
                        links.remove(&key)
                    }
                    _ => None,
                }
            };
            if let Some(link) = link {
                let _ = link.peripheral.disconnect().await;
                log::debug!("released idle BLE link to {key}");
            }
        });
    }

    /// Disconnect and drop a pooled link, releasing the device so it advertises
    /// again. The daemon keeps links open for reuse, but one-shot callers should
    /// release the device when done.
    pub async fn disconnect(&self, ble_addr: &str) {
        let key = ble_addr.to_uppercase();
        if let Some(link) = self.inner.links.lock().await.remove(&key) {
            let _ = link.peripheral.disconnect().await;
        }
    }

    /// Release every pooled link at shutdown, disconnecting each peripheral so
    /// the device advertises again for other centrals (the phone app) right away
    /// instead of waiting out BlueZ's link-supervision timeout. Marks the client
    /// shutting down first so the status readers don't reconnect a link we just
    /// dropped.
    pub async fn disconnect_all(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        let links: Vec<Arc<DeviceLink>> = {
            let mut map = self.inner.links.lock().await;
            map.drain().map(|(_, link)| link).collect()
        };
        for link in links {
            if let Err(err) = link.peripheral.disconnect().await {
                log::debug!("BLE disconnect on shutdown failed: {err:#}");
            }
        }
    }

    /// Scan and list discovered peripherals as (address, local_name). Read-only
    /// discovery: no connect, no writes. Used by the `ble-probe` command to find a
    /// device's MAC.
    pub async fn scan_list(&self) -> anyhow::Result<Vec<(String, Option<String>)>> {
        let adapter = &self.inner.adapter;
        adapter
            .start_scan(ScanFilter::default())
            .await
            .context("BLE scan")?;
        sleep(Duration::from_secs(SCAN_SECS)).await;
        let _ = adapter.stop_scan().await;
        let mut out = Vec::new();
        for p in adapter.peripherals().await? {
            let name = p
                .properties()
                .await
                .ok()
                .flatten()
                .and_then(|pr| pr.local_name);
            out.push((p.address().to_string(), name));
        }
        Ok(out)
    }

    /// Read-only diagnostic: find the device, connect, discover services, and read
    /// BgcInfo. Writes nothing to the device and does not change its state. Used by
    /// `ble-probe` to confirm reachability and the encryption version before any
    /// control is attempted.
    pub async fn probe(&self, ble_addr: &str) -> anyhow::Result<ProbeReport> {
        let peripheral = self
            .find_peripheral(&ble_addr.to_uppercase())
            .await?
            .with_context(|| format!("BLE device {ble_addr} not found"))?;

        // Capture the advertisement before connecting; the Govee broadcast carries
        // the supportEncryption flag the app keys its encrypt-vs-plaintext decision
        // on (BleBroadCastInfo).
        let (manufacturer_data, service_data) = match peripheral.properties().await.ok().flatten() {
            Some(p) => (
                p.manufacturer_data.into_iter().collect(),
                p.service_data.into_iter().collect(),
            ),
            None => (Vec::new(), Vec::new()),
        };

        // The GATT half is best-effort: a transient connect failure (BlueZ
        // le-connection-abort-by-local is common) shouldn't discard the
        // advertisement we already have, since that carries the encryption flag.
        let (characteristics, bgc_info) = match Self::connect_discover_read(&peripheral).await {
            Ok(result) => result,
            Err(err) => {
                log::warn!("BLE probe GATT step failed ({err:#}); reporting advertisement only");
                (Vec::new(), None)
            }
        };
        let version = bgc_info.as_deref().and_then(negotiate_version);

        let _ = peripheral.disconnect().await;
        Ok(ProbeReport {
            address: peripheral.address().to_string(),
            manufacturer_data,
            service_data,
            characteristics,
            bgc_info,
            version,
        })
    }

    /// Connect, discover services, and read BgcInfo if present. Returns the full
    /// characteristic list and the BgcInfo bytes. Used only by `probe`.
    async fn connect_discover_read(
        peripheral: &Peripheral,
    ) -> anyhow::Result<(Vec<(Uuid, Uuid, CharPropFlags)>, Option<Vec<u8>>)> {
        connect_with_retry(peripheral).await?;
        peripheral
            .discover_services()
            .await
            .context("BLE discover services")?;

        let characteristics = peripheral
            .characteristics()
            .into_iter()
            .map(|c| (c.service_uuid, c.uuid, c.properties))
            .collect();

        let bgc_char = peripheral
            .characteristics()
            .into_iter()
            .find(|c| c.uuid == BGC_CHAR_UUID);
        let bgc_info = match bgc_char {
            Some(c) => peripheral.read(&c).await.ok(),
            None => None,
        };
        Ok((characteristics, bgc_info))
    }

    async fn get_or_connect(&self, ble_addr: &str) -> anyhow::Result<Arc<DeviceLink>> {
        anyhow::ensure!(
            !self.inner.shutting_down.load(Ordering::SeqCst),
            "BLE client is shutting down"
        );
        let key = ble_addr.to_uppercase();
        // clone the cached link out and drop the lock before awaiting on the
        // connection check, so the map mutex isn't held across an await.
        let cached = self.inner.links.lock().await.get(&key).cloned();
        if let Some(link) = cached
            && link.peripheral.is_connected().await.unwrap_or(false)
        {
            return Ok(link);
        }
        let link = Arc::new(self.connect_and_handshake(&key).await?);
        self.inner.links.lock().await.insert(key, link.clone());
        Ok(link)
    }

    async fn connect_and_handshake(&self, ble_addr_upper: &str) -> anyhow::Result<DeviceLink> {
        let peripheral = self
            .find_peripheral(ble_addr_upper)
            .await?
            .with_context(|| format!("BLE device {ble_addr_upper} not found"))?;

        connect_with_retry(&peripheral).await?;
        peripheral
            .discover_services()
            .await
            .context("BLE discover services")?;

        let chars = peripheral.characteristics();
        let find = |uuid: Uuid| {
            chars
                .iter()
                .find(|c| c.uuid == uuid && c.service_uuid == SERVICE_UUID)
                .cloned()
        };
        let data_char =
            find(DATA_CHAR_UUID).context("device missing the BLE data characteristic")?;
        // The device replies to the handshake (and sends status) as notifications.
        // The captured session shows them on the 2b10 notify char; subscribe to it
        // and to the data char, and accept replies from either.
        let notify_chars: Vec<Characteristic> = [NOTIFY_CHAR_UUID, DATA_CHAR_UUID]
            .iter()
            .filter_map(|u| find(*u))
            .collect();

        // Pick the version: BgcInfo present -> read it (V1/V2); absent -> assume V1
        // (the app defaults to V1 when a device supports encryption but not V2).
        let version = match find(BGC_CHAR_UUID) {
            Some(bgc) => {
                let info = peripheral.read(&bgc).await.context("read BgcInfo")?;
                negotiate_version(&info)
                    .with_context(|| format!("unknown BLE encrypt version {info:?}"))?
            }
            None => Version::V1,
        };

        // Run the session handshake. If it fails on a device with no BgcInfo, the
        // device is likely an older unencrypted one, so fall back to plaintext.
        let session = match handshake(&peripheral, &data_char, &notify_chars, version).await {
            Ok(session) => session,
            Err(err) if find(BGC_CHAR_UUID).is_none() => {
                log::info!("V1 handshake failed ({err:#}); falling back to plaintext frames");
                Session::Plaintext
            }
            Err(err) => return Err(err),
        };
        Ok(DeviceLink {
            peripheral,
            data_char,
            session: Mutex::new(session),
            generation: AtomicU64::new(0),
            held: AtomicBool::new(false),
        })
    }

    /// Find a peripheral by BLE MAC. Scans in a retry loop because Govee devices
    /// advertise intermittently (and go quiet for a while after a connection), so a
    /// single short scan often misses them. Checks BlueZ's known set first, then
    /// runs up to SCAN_ATTEMPTS scan windows, returning as soon as the device
    /// appears.
    async fn find_peripheral(&self, ble_addr_upper: &str) -> anyhow::Result<Option<Peripheral>> {
        let adapter = &self.inner.adapter;
        if let Some(p) = match_address(adapter, ble_addr_upper).await? {
            return Ok(Some(p));
        }
        adapter
            .start_scan(ScanFilter::default())
            .await
            .context("BLE scan")?;
        let mut found = None;
        for _ in 0..SCAN_ATTEMPTS {
            sleep(Duration::from_secs(SCAN_SECS)).await;
            if let Some(p) = match_address(adapter, ble_addr_upper).await? {
                found = Some(p);
                break;
            }
        }
        let _ = adapter.stop_scan().await;
        Ok(found)
    }

    /// Hold a persistent link to one device and stream its state. After the
    /// handshake it sends the SKU's aa-read status burst, then keeps the link
    /// open with the aa00 keepalive and re-reads on a slower cadence. Inbound aa
    /// notifications are decoded for the SKU and folded into held state. Runs
    /// until the task is dropped, reconnecting after a drop. Holding the link
    /// blocks other BLE centrals (the phone app) from the device, which is the
    /// cost of continuous cloud-free state.
    pub async fn run_status_reader(
        &self,
        state: StateHandle,
        sku: String,
        device_id: String,
        ble_addr: String,
    ) {
        let read_frames = govee_api::ble::status_read_frames(&sku);
        if read_frames.is_empty() {
            return;
        }
        loop {
            if let Err(err) = self
                .reader_session(&state, &sku, &device_id, &ble_addr, &read_frames)
                .await
            {
                log::warn!("BLE reader for {device_id} ({ble_addr}) ended: {err:#}");
            }
            sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
        }
    }

    /// One connected session of the status reader: establish (and hold) the link,
    /// open the notification stream, send the read burst, then loop on inbound
    /// notifications and the keepalive timer until the link drops or a write
    /// fails.
    async fn reader_session(
        &self,
        state: &StateHandle,
        sku: &str,
        device_id: &str,
        ble_addr: &str,
        read_frames: &[Vec<u8>],
    ) -> anyhow::Result<()> {
        let link = self.get_or_connect(ble_addr).await?;
        // Mark the link held so a stale idle timer from an earlier command can't
        // disconnect it out from under the keepalive.
        link.held.store(true, Ordering::SeqCst);

        // The device replies to reads and the keepalive as notifications. Subscribe
        // to the notify and data chars (replies arrive on the notify char) and open
        // a stream for the link lifetime.
        let notify_chars: Vec<Characteristic> = link
            .peripheral
            .characteristics()
            .into_iter()
            .filter(|c| {
                c.service_uuid == SERVICE_UUID
                    && (c.uuid == NOTIFY_CHAR_UUID || c.uuid == DATA_CHAR_UUID)
            })
            .collect();
        for c in &notify_chars {
            link.peripheral
                .subscribe(c)
                .await
                .with_context(|| format!("subscribe to BLE notifications on {}", c.uuid))?;
        }
        let mut notifications = link
            .peripheral
            .notifications()
            .await
            .context("open BLE notification stream")?;

        self.write_recorded(state, sku, device_id, &link, read_frames)
            .await
            .context("BLE status read burst")?;
        let mut last_read = Instant::now();

        let heartbeat = [HEARTBEAT_FRAME.to_vec()];
        let mut beat = interval(Duration::from_secs(HEARTBEAT_SECS));
        // interval fires immediately on the first tick; skip it so the first
        // keepalive is one interval after the burst we just sent.
        beat.tick().await;

        loop {
            tokio::select! {
                notification = notifications.next() => {
                    let Some(notification) = notification else {
                        anyhow::bail!("notification stream ended");
                    };
                    self.handle_notification(state, sku, device_id, &link, &notification.value)
                        .await?;
                }
                _ = beat.tick() => {
                    self.write_recorded(state, sku, device_id, &link, &heartbeat)
                        .await
                        .context("BLE keepalive")?;
                    if last_read.elapsed() >= Duration::from_secs(READ_REFRESH_SECS) {
                        self.write_recorded(state, sku, device_id, &link, read_frames)
                            .await
                            .context("BLE status re-read")?;
                        last_read = Instant::now();
                    }
                }
            }
        }
    }

    /// Record each outbound frame on the inspector (Out/Ble), then write them to
    /// the link. The reader's reads and keepalive go out this way so the inspector
    /// shows the sends, not just the device's echoes.
    async fn write_recorded(
        &self,
        state: &StateHandle,
        sku: &str,
        device_id: &str,
        link: &Arc<DeviceLink>,
        frames: &[Vec<u8>],
    ) -> anyhow::Result<()> {
        for frame in frames {
            state.notify_frame(
                device_id,
                sku,
                FrameDirection::Out,
                FrameTransport::Ble,
                hex_pretty(frame),
            );
        }
        write_to_link(link, frames).await
    }

    /// Decrypt one inbound notification, decode it for the SKU, fold it into held
    /// state, and publish if it changed anything. A frame that fails to decrypt or
    /// decodes to nothing actionable (the keepalive echo, an undecoded frame) is
    /// recorded on the inspector but changes no state.
    async fn handle_notification(
        &self,
        state: &StateHandle,
        sku: &str,
        device_id: &str,
        link: &Arc<DeviceLink>,
        raw: &[u8],
    ) -> anyhow::Result<()> {
        let frame = {
            let session = link.session.lock().await;
            session.decrypt_notification(raw)
        };
        let Some(frame) = frame else {
            log::debug!("BLE notify from {device_id} failed to decrypt: {raw:02x?}");
            return Ok(());
        };
        state.notify_frame(
            device_id,
            sku,
            FrameDirection::In,
            FrameTransport::Ble,
            hex_pretty(&frame),
        );
        let decoded = govee_api::ble::decode_frame(sku, &frame);
        let changed = state
            .device_mut(sku, device_id)
            .await
            .apply_ble_status(&decoded);
        if changed {
            state.notify_of_state_change(device_id).await?;
        }
        Ok(())
    }
}

/// Encrypt each frame under the link's session and write it to the data
/// characteristic. Shared by the command path and the status reader.
async fn write_to_link(link: &DeviceLink, frames: &[Vec<u8>]) -> anyhow::Result<()> {
    let mut session = link.session.lock().await;
    for frame in frames {
        let wire = session.encrypt_command(frame);
        link.peripheral
            .write(&link.data_char, &wire, WriteType::WithoutResponse)
            .await
            .context("writing BLE command frame")?;
    }
    Ok(())
}

/// Reconcile status readers against the device set: for each BLE device with a
/// local read path that doesn't already have a reader, spawn one. Runs forever,
/// re-scanning periodically so devices discovered after startup get a reader too.
pub async fn run_reader_manager(state: StateHandle, ble: BleClient) {
    let mut active: HashSet<String> = HashSet::new();
    loop {
        for device in state.devices().await {
            let Some(addr) = device.ble_address() else {
                continue;
            };
            if govee_api::ble::status_read_frames(&device.sku).is_empty() {
                continue;
            }
            if active.insert(device.id.clone()) {
                let addr = addr.to_string();
                log::info!(
                    "Starting BLE status reader for {id} ({addr})",
                    id = device.id
                );
                let ble = ble.clone();
                let state = state.clone();
                let sku = device.sku.clone();
                let id = device.id.clone();
                tokio::spawn(async move {
                    ble.run_status_reader(state, sku, id, addr).await;
                });
            }
        }
        sleep(Duration::from_secs(READER_RECONCILE_SECS)).await;
    }
}

/// Connect to a peripheral, retrying the transient BlueZ
/// `le-connection-abort-by-local` a couple of times before giving up.
async fn connect_with_retry(peripheral: &Peripheral) -> anyhow::Result<()> {
    if peripheral.is_connected().await.unwrap_or(false) {
        return Ok(());
    }
    let mut last_err = None;
    for attempt in 1..=3 {
        match peripheral.connect().await {
            Ok(()) => return Ok(()),
            Err(err) => {
                log::debug!("BLE connect attempt {attempt} failed: {err}");
                last_err = Some(err);
                sleep(Duration::from_millis(500)).await;
            }
        }
    }
    Err(anyhow::Error::new(last_err.expect("loop ran at least once")).context("BLE connect"))
}

async fn match_address(
    adapter: &Adapter,
    ble_addr_upper: &str,
) -> anyhow::Result<Option<Peripheral>> {
    for p in adapter.peripherals().await? {
        if p.address().to_string().to_uppercase() == ble_addr_upper {
            return Ok(Some(p));
        }
    }
    Ok(None)
}

/// Run the V1 or V2 session handshake: write the request to the data char, await
/// the reply on the notify chars, and build the session. Subscribes to every
/// notify char and matches the reply by content (the device may reply on a
/// different characteristic than the one we write to).
async fn handshake(
    peripheral: &Peripheral,
    data_char: &Characteristic,
    notify_chars: &[Characteristic],
    version: Version,
) -> anyhow::Result<Session> {
    for c in notify_chars {
        peripheral
            .subscribe(c)
            .await
            .with_context(|| format!("subscribe to BLE notifications on {}", c.uuid))?;
    }
    let mut notifications = peripheral
        .notifications()
        .await
        .context("open BLE notification stream")?;

    match version {
        Version::V1 => {
            peripheral
                .write(data_char, &v1_build_request(), WriteType::WithoutResponse)
                .await?;
            let reply = next_reply(&mut notifications).await?;
            let key = v1_parse_key_reply(&reply).context("invalid V1 session-key reply")?;
            peripheral
                .write(data_char, &v1_build_confirm(), WriteType::WithoutResponse)
                .await?;
            let ack = next_reply(&mut notifications).await?;
            anyhow::ensure!(
                v1_is_confirm_ack(&ack),
                "V1 session-key confirm was not acked"
            );
            Ok(Session::v1(key))
        }
        Version::V2 => {
            let iv_send = random_iv_send();
            peripheral
                .write(
                    data_char,
                    &v2_build_request(&iv_send),
                    WriteType::WithoutResponse,
                )
                .await?;
            let reply = next_reply(&mut notifications).await?;
            let payload = v2_parse_single_reply(&reply).context("invalid V2 session-key reply")?;
            v2_session_from_reply(iv_send, &payload).context("V2 session-key derivation failed")
        }
    }
}

/// Pull the next notification (from any subscribed characteristic) with a timeout.
async fn next_reply<S>(stream: &mut S) -> anyhow::Result<Vec<u8>>
where
    S: Stream<Item = ValueNotification> + Unpin,
{
    match timeout(Duration::from_secs(REPLY_TIMEOUT_SECS), stream.next()).await {
        Ok(Some(n)) => Ok(n.value),
        Ok(None) => anyhow::bail!("BLE notification stream ended during handshake"),
        Err(_) => anyhow::bail!("timed out waiting for BLE device reply"),
    }
}
