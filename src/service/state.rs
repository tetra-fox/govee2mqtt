use crate::hass_mqtt::topic::Topics;
use crate::service::ble::BleClient;
use crate::service::control::controller_for;
use crate::service::coordinator::Coordinator;
use crate::service::device::Device;
use crate::service::hass::{HassClient, topic_safe_id};
use crate::service::iot::IotClient;
use anyhow::Context;
use govee_api::lan_api::Client as LanClient;
use govee_api::platform_api::{
    DeviceCapability, DeviceType, GoveeApiClient, sort_and_dedup_scenes,
};
use govee_api::temperature::{TemperatureScale, TemperatureValue};
use govee_api::undoc_api::GoveeUndocumentedApi;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{MappedMutexGuard, Mutex, MutexGuard, Notify, Semaphore};

#[derive(Default)]
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
    /// registration, keyed by device config topic, then by component unique id,
    /// with the value being the component's platform. Held so the next
    /// registration can: clear a device topic we no longer produce (empty
    /// retained payload removes the whole device), and tombstone a single
    /// component that a still-present device no longer produces (republished as
    /// `{"p": platform}`, home assistant's signal to drop just that component).
    published_components: Mutex<PublishedComponents>,
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
}

pub type StateHandle = Arc<State>;

/// Device config topic -> (component unique id -> platform), the discovery
/// components published in one registration pass.
pub type PublishedComponents = HashMap<String, HashMap<String, String>>;

impl State {
    pub fn new() -> Self {
        Self::default()
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

    pub async fn set_base_topic(&self, base_topic: String) {
        *self.base_topic.lock().await = base_topic;
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
            log::info!("requesting update via IoT MQTT {device} {device_state:?}");
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
            log::info!("requesting update via Platform API {device} {device_state:?}");
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
        crate::service::transport::device_control(self, device, capability, value).await
    }

    /// Try to control `instance` as an IoT ptReal frame; returns false if it
    /// isn't a frame-encoded instance for this SKU (caller falls back).
    pub async fn try_iot_capability(
        self: &Arc<Self>,
        device: &Device,
        instance: &str,
        value: &JsonValue,
    ) -> anyhow::Result<bool> {
        crate::service::transport::try_iot_capability(self, device, instance, value).await
    }

    pub async fn device_light_power_on(
        self: &Arc<Self>,
        device: &Device,
        on: bool,
    ) -> anyhow::Result<()> {
        controller_for(device)
            .light_power_on(self, device, on)
            .await
    }

    pub async fn device_power_on(
        self: &Arc<Self>,
        device: &Device,
        on: bool,
    ) -> anyhow::Result<()> {
        controller_for(device).power_on(self, device, on).await
    }

    pub async fn device_set_brightness(
        self: &Arc<Self>,
        device: &Device,
        percent: u8,
    ) -> anyhow::Result<()> {
        controller_for(device)
            .set_brightness(self, device, percent)
            .await
    }

    pub async fn device_set_color_temperature(
        self: &Arc<Self>,
        device: &Device,
        kelvin: u32,
    ) -> anyhow::Result<()> {
        controller_for(device)
            .set_color_temperature(self, device, kelvin)
            .await
    }

    pub async fn device_set_color_rgb(
        self: &Arc<Self>,
        device: &Device,
        r: u8,
        g: u8,
        b: u8,
    ) -> anyhow::Result<()> {
        controller_for(device)
            .set_color_rgb(self, device, r, g, b)
            .await
    }

    pub async fn humidifier_set_parameter(
        self: &Arc<Self>,
        device: &Device,
        work_mode: i64,
        value: i64,
    ) -> anyhow::Result<()> {
        crate::service::transport::humidifier_set_parameter(self, device, work_mode, value).await
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
        crate::service::transport::fan_set_speed(self, device, work_mode, speed).await
    }

    /// Switch a single outlet of a multi-outlet socket (eg: H5082).
    /// <https://github.com/wez/govee2mqtt/issues/65>
    pub async fn device_set_socket_outlet(
        self: &Arc<Self>,
        device: &Device,
        index: u8,
        on: bool,
    ) -> anyhow::Result<()> {
        crate::service::transport::socket_turn(self, device, index, on).await
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
        if let Some(client) = self.get_platform_client().await
            && let Some(info) = &device.http_device_info
        {
            log::info!("Using Platform API to set {device} target temperature to {target}");
            client
                .set_target_temperature(info, instance_name, target, None)
                .await?;
            return Ok(());
        }

        anyhow::bail!("Unable to set temperature for {device}");
    }

    pub async fn device_set_scene(
        self: &Arc<Self>,
        device: &Device,
        scene: &str,
    ) -> anyhow::Result<()> {
        crate::service::transport::device_set_scene(self, device, scene).await
    }

    // Take care not to call this while you hold a mutable device
    // reference, as that will deadlock!
    pub async fn notify_of_state_change(self: &Arc<Self>, device_id: &str) -> anyhow::Result<()> {
        let Some(canonical_device) = self.device_by_id(device_id).await else {
            anyhow::bail!("cannot find device {device_id}!?");
        };

        if let Some(hass) = self.get_hass_client().await {
            hass.advise_hass_of_light_state(&canonical_device, self)
                .await?;
        }

        Ok(())
    }
}
