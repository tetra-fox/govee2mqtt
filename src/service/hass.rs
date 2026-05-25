use crate::hass_mqtt::climate::mqtt_set_temperature;
use crate::hass_mqtt::enumerator::{enumerate_all_entites, enumerate_entities_for_device};
use crate::hass_mqtt::humidifier::{mqtt_device_set_work_mode, mqtt_humidifier_set_target};
use crate::hass_mqtt::instance::EntityList;
use crate::hass_mqtt::number::{
    mqtt_capability_number_command, mqtt_music_sensitivity_command, mqtt_number_command,
};
use crate::hass_mqtt::router::{Message, MqttRouter, Params, Payload, State};
use crate::hass_mqtt::select::{mqtt_set_capability_mode, mqtt_set_mode_scene};
use crate::hass_mqtt::switch::mqtt_music_auto_color_command;
use crate::service::device::Device as ServiceDevice;
use crate::service::state::StateHandle;
use anyhow::Context;
use govee_api::http::from_json;
use govee_api::lan_api::DeviceColor;
use govee_api::opt_env_var;
use govee_api::platform_api::DeviceType;
use govee_api::temperature::TemperatureScale;
use rumqttc::{AsyncClient, Event, EventLoop, LastWill, MqttOptions, Packet, QoS};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

const HASS_REGISTER_DELAY: tokio::time::Duration = tokio::time::Duration::from_secs(15);

#[derive(clap::Parser, Debug)]
pub struct HassArguments {
    /// The mqtt broker hostname or address.
    /// You may also set this via the GOVEE_MQTT_HOST environment variable.
    #[arg(long, global = true)]
    mqtt_host: Option<String>,

    /// The mqtt broker port
    /// You may also set this via the GOVEE_MQTT_PORT environment variable.
    /// If unspecified, uses 1883
    #[arg(long, global = true)]
    mqtt_port: Option<u16>,

    /// The username to authenticate against the broker
    /// You may also set this via the GOVEE_MQTT_USER environment variable.
    #[arg(long, global = true)]
    mqtt_username: Option<String>,

    /// The password to authenticate against the broker
    /// You may also set this via the GOVEE_MQTT_PASSWORD environment variable.
    #[arg(long, global = true)]
    mqtt_password: Option<String>,

    /// The base topic, used as the prefix for all MQTT topics and as the
    /// prefix for the Home Assistant entity unique ids.
    /// You may also set this via the GOVEE_MQTT_BASE_TOPIC environment variable.
    /// If unspecified, uses "govee2mqtt". Set this to "gv2mqtt" to keep the
    /// topics and entities from an upstream wez/govee2mqtt install.
    #[arg(long, global = true)]
    mqtt_base_topic: Option<String>,

    #[arg(long, global = true, default_value = "homeassistant")]
    hass_discovery_prefix: String,

    /// The temperature scale to use when showing temperature values as
    /// entities in home assistant. Can be either "C" or "F" for Celsius
    /// or Fahrenheit respectively.
    /// You may also set this vai the GOVEE_TEMPERATURE_SCALE environment
    /// variable.
    #[arg(long, global = true)]
    temperature_scale: Option<String>,
}

impl HassArguments {
    pub fn opt_mqtt_host(&self) -> anyhow::Result<Option<String>> {
        match &self.mqtt_host {
            Some(h) => Ok(Some(h.to_string())),
            None => opt_env_var("GOVEE_MQTT_HOST"),
        }
    }

    pub fn mqtt_host(&self) -> anyhow::Result<String> {
        self.opt_mqtt_host()?.ok_or_else(|| {
            anyhow::anyhow!(
                "Please specify the mqtt broker either via the \
                --mqtt-host parameter or by setting $GOVEE_MQTT_HOST"
            )
        })
    }

    pub fn mqtt_port(&self) -> anyhow::Result<u16> {
        match self.mqtt_port {
            Some(p) => Ok(p),
            None => Ok(opt_env_var("GOVEE_MQTT_PORT")?.unwrap_or(1883)),
        }
    }

    pub fn mqtt_username(&self) -> anyhow::Result<Option<String>> {
        match self.mqtt_username.clone() {
            Some(u) => Ok(Some(u)),
            None => opt_env_var("GOVEE_MQTT_USER"),
        }
    }

    pub fn mqtt_password(&self) -> anyhow::Result<Option<String>> {
        match self.mqtt_password.clone() {
            Some(u) => Ok(Some(u)),
            None => opt_env_var("GOVEE_MQTT_PASSWORD"),
        }
    }

    pub fn base_topic(&self) -> anyhow::Result<String> {
        match &self.mqtt_base_topic {
            Some(t) => Ok(t.to_string()),
            None => {
                Ok(opt_env_var("GOVEE_MQTT_BASE_TOPIC")?
                    .unwrap_or_else(|| "govee2mqtt".to_string()))
            }
        }
    }

    pub fn temperature_scale(&self) -> anyhow::Result<TemperatureScale> {
        match &self.temperature_scale {
            Some(s) => Ok(s.parse()?),
            None => {
                Ok(opt_env_var("GOVEE_TEMPERATURE_SCALE")?.unwrap_or(TemperatureScale::Celsius))
            }
        }
    }
}

#[derive(Clone)]
pub struct HassClient {
    client: AsyncClient,
}

impl HassClient {
    async fn register_with_hass(&self, state: &StateHandle) -> anyhow::Result<()> {
        // Serialize registration passes so the snapshot/diff of published
        // config topics below stays consistent if two passes overlap.
        let _guard = state.lock_registration().await;

        // Snapshot what we published last time and reset the recorded set; the
        // pass below repopulates it via record_published_config_topic.
        let previous_topics = state.take_published_config_topics().await;

        let enumeration = enumerate_all_entites(state).await?;
        let entities = &enumeration.entities;

        // Register the configs
        log::trace!("register_with_hass: register entities");
        entities.publish_config(state, self).await?;

        // Remove any entity we published before but no longer produce. Empty
        // retained payload is home assistant's signal to drop the entity. Only
        // safe when enumeration was complete: a partial pass (eg: the undoc
        // one-click API failed) is missing entities that still exist, and GCing
        // against it would wrongly delete them.
        if enumeration.complete {
            let current_topics = state.current_published_config_topics().await;
            for topic in previous_topics.difference(&current_topics) {
                log::info!("Removing stale discovery config {topic}");
                self.remove_config(topic)
                    .await
                    .with_context(|| format!("remove stale config {topic}"))?;
            }
        } else {
            log::info!(
                "Enumeration was incomplete; skipping stale-config cleanup to avoid removing entities that still exist"
            );
            // Carry the previous topics forward so a later complete pass can
            // still GC anything that legitimately went away. take_* cleared the
            // set; the pass above only re-recorded what it managed to produce.
            for topic in previous_topics {
                state.record_published_config_topic(topic).await;
            }
        }

        // Allow hass extra time to register the entities before
        // we mark them as available
        let delay = tokio::time::Duration::from_millis((10 * entities.len()) as u64);
        log::info!(
            "Wait {delay:?} for hass to settle on {} entity configs",
            entities.len()
        );
        tokio::time::sleep(delay).await;

        // Mark the bridge available. Retained so a late-subscribing hass (eg:
        // one that restarts after us) sees us as online without waiting for the
        // next registration.
        log::trace!("register_with_hass: mark as online");
        self.publish_availability(state.topics().await.availability(), "online")
            .await
            .context("online -> availability_topic")?;

        // Seed each device's per-device availability so entities resolve to a
        // concrete state right after discovery instead of waiting for the first
        // state change or the periodic sweep. Only controllable devices get
        // entities (see enumerate_entities_for_device), so only they have an
        // availability topic worth publishing.
        for device in state.devices().await {
            if device.is_controllable() {
                self.publish_device_availability(&device, state)
                    .await
                    .with_context(|| format!("device availability for {device}"))?;
            }
        }

        // report initial state
        log::trace!("register_with_hass: reporting state");
        entities
            .notify_state(state, self)
            .await
            .context("notify_state")?;

        log::trace!("register_with_hass: done");

        Ok(())
    }

    pub async fn publish<T: AsRef<str> + std::fmt::Display, P: AsRef<[u8]> + std::fmt::Display>(
        &self,
        topic: T,
        payload: P,
    ) -> anyhow::Result<()> {
        log::trace!("{topic} -> {payload}");
        self.client
            .publish(topic.as_ref(), QoS::AtMostOnce, false, payload.as_ref())
            .await?;
        Ok(())
    }

    pub async fn publish_obj<T: AsRef<str> + std::fmt::Display, P: Serialize>(
        &self,
        topic: T,
        payload: P,
    ) -> anyhow::Result<()> {
        let payload = serde_json::to_string(&payload)?;
        log::trace!("{topic} -> {payload}");
        self.client
            .publish(topic.as_ref(), QoS::AtMostOnce, false, payload)
            .await?;
        Ok(())
    }

    /// Publish a discovery config retained, so home assistant re-reads it from
    /// the broker on restart without waiting for us to notice and re-register.
    /// QoS 1 because losing a config silently drops the entity.
    pub async fn publish_config<T: AsRef<str> + std::fmt::Display, P: Serialize>(
        &self,
        topic: T,
        payload: P,
    ) -> anyhow::Result<()> {
        let payload = serde_json::to_string(&payload)?;
        log::trace!("{topic} -> {payload}");
        self.client
            .publish(topic.as_ref(), QoS::AtLeastOnce, true, payload)
            .await?;
        Ok(())
    }

    /// Publish an availability state retained, matching the retained last-will
    /// so the broker keeps the latest online/offline status for subscribers
    /// that connect after we do.
    pub async fn publish_availability<
        T: AsRef<str> + std::fmt::Display,
        P: AsRef<[u8]> + std::fmt::Display,
    >(
        &self,
        topic: T,
        payload: P,
    ) -> anyhow::Result<()> {
        log::trace!("{topic} -> {payload}");
        self.client
            .publish(topic.as_ref(), QoS::AtLeastOnce, true, payload.as_ref())
            .await?;
        Ok(())
    }

    /// Publish an empty retained payload to a discovery config topic, which is
    /// home assistant's signal to remove the entity. Used to clean up entities
    /// that a registration pass no longer produces.
    pub async fn remove_config<T: AsRef<str> + std::fmt::Display>(
        &self,
        topic: T,
    ) -> anyhow::Result<()> {
        log::trace!("{topic} -> <remove>");
        self.client
            .publish(topic.as_ref(), QoS::AtLeastOnce, true, "")
            .await?;
        Ok(())
    }

    pub async fn advise_hass_of_light_state(
        &self,
        device: &ServiceDevice,
        state: &StateHandle,
    ) -> anyhow::Result<()> {
        self.publish_device_availability(device, state).await?;

        let mut entities = EntityList::new();
        enumerate_entities_for_device(device, state, &mut entities).await?;
        entities.notify_state(state, self).await?;

        Ok(())
    }

    /// Publish a device's reachability to its per-device availability topic.
    /// Retained so a late-subscribing hass sees the last known status, matching
    /// the global availability and last-will.
    ///
    /// A device with no reachability signal uses bridge-only availability (see
    /// EntityConfig::device_availability) and has no per-device topic, so
    /// there's nothing to publish.
    pub async fn publish_device_availability(
        &self,
        device: &ServiceDevice,
        state: &StateHandle,
    ) -> anyhow::Result<()> {
        if !device.has_reachability_signal() {
            return Ok(());
        }
        let topic = state.topics().await.device_availability(device);
        let payload = device.availability_status().as_mqtt_payload();
        self.publish_availability(topic, payload).await
    }
}

pub fn topic_safe_string(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        if c == ':' || c == ' ' || c == '\\' || c == '/' || c == '\'' || c == '"' {
            result.push('_');
        } else {
            result.push(c.to_ascii_lowercase());
        }
    }
    result
}

pub fn topic_safe_id(device: &ServiceDevice) -> String {
    let mut id = device.id.to_string();
    id.retain(|c| c != ':');
    id.retain(|c| c != ' ');
    id
}

#[derive(Deserialize)]
pub struct IdParameter {
    pub id: String,
}

/// Someone clicked the "Request Platform API State" button
async fn mqtt_request_platform_data(
    Params(IdParameter { id }): Params<IdParameter>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    let device = state.resolve_device_read_only(&id).await?;
    log::info!("Request Platform API State for {device}");
    if !state.poll_platform_api(&device).await? {
        log::warn!("Unable to poll platform API for {device}");
    }
    Ok(())
}

#[derive(Deserialize, Debug, Clone)]
struct HassLightCommand {
    state: String,
    color_temp_kelvin: Option<u32>,
    color: Option<DeviceColor>,
    effect: Option<String>,
    brightness: Option<u8>,
}

/// HASS is sending a command to a light
async fn mqtt_light_command(
    Payload(payload): Payload<String>,
    Params(IdParameter { id }): Params<IdParameter>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    let device = state.resolve_device_for_control(&id).await?;

    let command: HassLightCommand = serde_json::from_str(&payload)?;
    log::info!("Command for {device}: {payload}");

    let is_light = device.device_type() == DeviceType::Light;

    if command.state == "OFF" {
        if is_light {
            state
                .device_light_power_on(&device, false)
                .await
                .context("mqtt_light_command: state.device_power_on")?;
        } else {
            state
                .device_set_brightness(&device, 0)
                .await
                .context("mqtt_light_command: state.device_set_brightness")?;
        }
    } else {
        let mut power_on = true;

        if let Some(brightness) = command.brightness {
            state
                .device_set_brightness(&device, brightness)
                .await
                .context("mqtt_light_command: state.device_set_brightness")?;
            power_on = false;
        }

        if let Some(effect) = &command.effect {
            state
                .device_set_scene(&device, effect)
                .await
                .context("mqtt_light_command: state.device_set_scene")?;
            // It doesn't make sense to vary color properties
            // at the same time as the scene properties, so
            // ignore those.
            // Brightness, set above, is ok.
            return Ok(());
        }

        if let Some(color) = &command.color {
            state
                .device_set_color_rgb(&device, color.r, color.g, color.b)
                .await
                .context("mqtt_light_command: state.device_set_color_rgb")?;
            power_on = false;
        }
        if let Some(kelvin) = command.color_temp_kelvin {
            state
                .device_set_color_temperature(&device, kelvin)
                .await
                .context("mqtt_light_command: state.device_set_color_temperature")?;
            power_on = false;
        }

        if power_on {
            if is_light {
                state
                    .device_light_power_on(&device, true)
                    .await
                    .context("mqtt_light_command: state.device_power_on")?;
            } else if command.brightness.is_none() {
                // The device is not primarily a light and we don't have
                // a guaranteed way to power it on without setting the
                // brightness to something, and we know we didn't set
                // the brightness just now, so let's turn it on 100%
                state
                    .device_set_brightness(&device, 100)
                    .await
                    .context("mqtt_light_command: state.device_set_brightness")?;
            }
        }
    }

    Ok(())
}

#[derive(Deserialize)]
struct IdAndSeg {
    id: String,
    segment: String,
}

async fn mqtt_light_segment_command(
    Payload(payload): Payload<String>,
    Params(IdAndSeg { id, segment }): Params<IdAndSeg>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    let device = state.resolve_device_for_control(&id).await?;
    let segment: u32 = segment.parse()?;

    let command: HassLightCommand = from_json(&payload)?;
    log::info!("Command for {device} segment {segment}: {payload}");

    if let Some(client) = state.get_platform_client().await {
        let info = device
            .http_device_info
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("HTTP device info is missing"))?;

        log::info!("Using Platform API to control {device} segment");

        if let Some(brightness) = command.brightness {
            client
                .set_segment_brightness(info, segment, brightness)
                .await?;
        } else if command.state == "OFF" {
            // Do nothing here. We used to set brightness to zero,
            // but it is problematic:
            // * Some devices don't have a 0
            // * Setting it to 0 will power up the rest of the device,
            //   so if HASS is turning off all lights in an area, the
            //   effect is that they will turn off and then immediate
            //   on again when there are segments involved
            // client.set_segment_brightness(&info, segment, 0).await?;
        }
        if let Some(color) = &command.color {
            client
                .set_segment_rgb(info, segment, color.r, color.g, color.b)
                .await?;
        }
    } else {
        anyhow::bail!("set segments for {device}: Platform API is not available");
    }

    Ok(())
}

async fn mqtt_purge_caches(State(state): State<StateHandle>) -> anyhow::Result<()> {
    log::info!("mqtt_purge_caches");
    govee_api::cache::purge_cache()?;
    state
        .get_hass_client()
        .await
        .expect("have hass client")
        .register_with_hass(&state)
        .await
        .context("register_with_hass")
}

async fn mqtt_oneclick(
    Payload(name): Payload<String>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    log::info!("mqtt_oneclick: {name}");

    let undoc = state
        .get_undoc_client()
        .await
        .ok_or_else(|| anyhow::anyhow!("Undoc API client is not available"))?;
    let items = undoc.parse_one_clicks().await?;
    let item = items
        .iter()
        .find(|item| item.name == name)
        .ok_or_else(|| anyhow::anyhow!("didn't find item {name}"))?;

    let iot = state
        .get_iot_client()
        .await
        .ok_or_else(|| anyhow::anyhow!("AWS IoT client is not available"))?;

    iot.activate_one_click(item).await
}

#[derive(Deserialize)]
struct IdAndInst {
    id: String,
    instance: String,
}

async fn mqtt_switch_command(
    Payload(command): Payload<String>,
    Params(IdAndInst { id, instance }): Params<IdAndInst>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    log::info!("{instance} for {id}: {command}");
    let device = state.resolve_device_for_control(&id).await?;

    let on = match command.as_str() {
        "ON" | "on" => true,
        "OFF" | "off" => false,
        _ => anyhow::bail!("invalid {command} for {id}"),
    };

    if instance == "powerSwitch" {
        state.device_power_on(&device, on).await?;
    } else if let Some(client) = state.get_platform_client().await {
        if let Some(http_dev) = &device.http_device_info {
            client.set_toggle_state(http_dev, &instance, on).await?;
        } else {
            anyhow::bail!("No platform state available to set {id} {instance} to {on}");
        }
    } else {
        anyhow::bail!("Don't know how to {command} for {id} {instance}!");
    }

    Ok(())
}

#[derive(Deserialize)]
struct IdAndOutlet {
    id: String,
    index: String,
}

/// HASS is sending a command to a single outlet of a multi-outlet socket
/// (eg: H5082). Routed over the IoT API, which is the only transport that can
/// address individual outlets.
/// <https://github.com/wez/govee2mqtt/issues/65>
async fn mqtt_outlet_command(
    Payload(command): Payload<String>,
    Params(IdAndOutlet { id, index }): Params<IdAndOutlet>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    log::info!("outlet {index} for {id}: {command}");
    let index: u8 = index.parse()?;
    let on = match command.as_str() {
        "ON" | "on" => true,
        "OFF" | "off" => false,
        _ => anyhow::bail!("invalid {command} for {id} outlet {index}"),
    };
    let device = state.resolve_device_for_control(&id).await?;

    state.device_set_socket_outlet(&device, index, on).await?;

    Ok(())
}

/// HASS is advising us that its status has changed
async fn mqtt_homeassitant_status(
    Payload(status): Payload<String>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    let client = state
        .get_hass_client()
        .await
        .expect("hass client to be present");

    log::info!(
        "Home Assistant status changed: {status}, waiting {HASS_REGISTER_DELAY:?} before re-registering entities"
    );
    tokio::time::sleep(HASS_REGISTER_DELAY).await;

    client.register_with_hass(&state).await?;

    Ok(())
}

/// Build the router (subscribing to every command topic) and register all
/// entities with home assistant.
///
/// rumqttc only writes queued subscribe/publish requests to the network while
/// `EventLoop::poll` is being driven, and its request channel is bounded. This
/// function issues many subscribes plus hundreds of config/state publishes with
/// sleeps in between, so it must NOT run inline in the poll loop: doing so parks
/// the event loop, the request channel fills, and the next publish blocks
/// forever. It runs in its own task (see `run_mqtt_loop`) so the poll loop keeps
/// draining the channel concurrently.
async fn build_router_and_register(
    client: &AsyncClient,
    state: &StateHandle,
    first_connect: bool,
) -> anyhow::Result<Arc<MqttRouter<StateHandle>>> {
    if first_connect {
        // Give LAN disco a chance to get current state before we register with
        // hass.
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    let router = {
        let disco_prefix = state.get_hass_disco_prefix().await;
        let topics = state.topics().await;
        let mut router: MqttRouter<StateHandle> = MqttRouter::new(client.clone());

        router
            .route(format!("{disco_prefix}/status"), mqtt_homeassitant_status)
            .await?;

        router
            .route(topics.route_light_command(), mqtt_light_command)
            .await?;
        router
            .route(
                topics.route_light_segment_command(),
                mqtt_light_segment_command,
            )
            .await?;
        router
            .route(topics.route_switch_command(), mqtt_switch_command)
            .await?;
        router
            .route(topics.route_outlet_command(), mqtt_outlet_command)
            .await?;

        router.route(topics.oneclick(), mqtt_oneclick).await?;
        router
            .route(topics.purge_caches(), mqtt_purge_caches)
            .await?;
        router
            .route(
                topics.route_request_platform_data(),
                mqtt_request_platform_data,
            )
            .await?;
        router
            .route(topics.route_number_command(), mqtt_number_command)
            .await?;
        router
            .route(
                topics.route_humidifier_set_mode(),
                mqtt_device_set_work_mode,
            )
            .await?;
        router
            .route(topics.route_set_work_mode(), mqtt_device_set_work_mode)
            .await?;
        router
            .route(
                topics.route_humidifier_set_target(),
                mqtt_humidifier_set_target,
            )
            .await?;
        router
            .route(topics.route_set_temperature(), mqtt_set_temperature)
            .await?;
        router
            .route(topics.route_set_mode_scene(), mqtt_set_mode_scene)
            .await?;
        router
            .route(
                topics.route_capability_number_command(),
                mqtt_capability_number_command,
            )
            .await?;
        router
            .route(
                topics.route_capability_mode_command(),
                mqtt_set_capability_mode,
            )
            .await?;
        router
            .route(
                topics.route_music_sensitivity_command(),
                mqtt_music_sensitivity_command,
            )
            .await?;
        router
            .route(
                topics.route_music_auto_color_command(),
                mqtt_music_auto_color_command,
            )
            .await?;

        router
    };

    tokio::time::sleep(HASS_REGISTER_DELAY).await;
    state
        .get_hass_client()
        .await
        .expect("have hass client")
        .register_with_hass(state)
        .await
        .context("register_with_hass")?;

    Ok(Arc::new(router))
}

async fn run_mqtt_loop(
    state: StateHandle,
    mut eventloop: EventLoop,
    client: AsyncClient,
) -> anyhow::Result<()> {
    // The router is (re)built on each ConnAck. rumqttc uses a clean session, so
    // the broker drops our subscriptions across a reconnect; rebuilding
    // re-subscribes and re-registers the entities with home assistant.
    //
    // Registration runs in a separate task (see build_router_and_register for
    // why it must not run inline here) and publishes the finished router back
    // over this watch channel; the poll loop reads it to dispatch messages.
    let (router_tx, router_rx) =
        tokio::sync::watch::channel::<Option<Arc<MqttRouter<StateHandle>>>>(None);
    let mut register_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut first_connect = true;

    loop {
        let event = match eventloop.poll().await {
            Ok(event) => event,
            Err(rumqttc::ConnectionError::RequestsDone) => {
                log::info!("MQTT request channel closed, loop terminating");
                return Ok(());
            }
            Err(err) => {
                // rumqttc reconnects on the next poll; the subscriptions are
                // gone until then, so drop the router and rebuild on ConnAck.
                log::warn!("MQTT disconnected: {err:#}");
                if let Some(task) = register_task.take() {
                    task.abort();
                }
                let _ = router_tx.send(None);
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        match event {
            Event::Incoming(Packet::ConnAck(_)) => {
                log::info!("MQTT connected");
                // Supersede any registration still in flight from a previous
                // connect that we never saw complete.
                if let Some(task) = register_task.take() {
                    task.abort();
                }
                let _ = router_tx.send(None);

                let was_first = first_connect;
                first_connect = false;
                let client = client.clone();
                let state = state.clone();
                let router_tx = router_tx.clone();
                register_task = Some(tokio::spawn(async move {
                    match build_router_and_register(&client, &state, was_first).await {
                        Ok(router) => {
                            let _ = router_tx.send(Some(router));
                        }
                        Err(err) => {
                            log::error!("registering with home assistant: {err:#}");
                        }
                    }
                }));
            }
            Event::Incoming(Packet::Publish(publish)) => {
                let Some(router) = router_rx.borrow().clone() else {
                    log::warn!(
                        "Received publish on {} before router was ready",
                        publish.topic
                    );
                    continue;
                };
                let message = Message {
                    topic: publish.topic,
                    payload: publish.payload.to_vec(),
                };
                let state = state.clone();
                tokio::spawn(async move {
                    let topic = message.topic.clone();
                    if let Err(err) = router.dispatch(message, state).await {
                        log::error!("While dispatching message on {topic}: {err:#}");
                    }
                });
            }
            _ => {}
        }
    }
}

pub async fn spawn_hass_integration(
    state: StateHandle,
    args: &HassArguments,
) -> anyhow::Result<()> {
    state.set_temperature_scale(args.temperature_scale()?).await;

    state.set_base_topic(args.base_topic()?).await;
    let topics = state.topics().await;

    let mqtt_host = args.mqtt_host()?;
    let mqtt_username = args.mqtt_username()?;
    let mqtt_password = args.mqtt_password()?;
    let mqtt_port = args.mqtt_port()?;

    let mut mqtt_options = MqttOptions::new(
        format!("govee2mqtt/{}", uuid::Uuid::new_v4().simple()),
        &mqtt_host,
        mqtt_port,
    );
    mqtt_options.set_keep_alive(Duration::from_secs(120));
    // Retained so a hass that subscribes after we have already disconnected
    // still sees us as offline, matching the retained "online" we publish on
    // registration.
    mqtt_options.set_last_will(LastWill::new(
        topics.availability(),
        "offline",
        QoS::AtLeastOnce,
        true,
    ));

    match (mqtt_username, mqtt_password) {
        (Some(user), Some(pass)) => {
            mqtt_options.set_credentials(user, pass);
        }
        (None, None) => {}
        _ => {
            log::error!(
                "MQTT username and password either both need to be set, or both need to be unset"
            );
        }
    }

    log::info!("Connecting to mqtt broker {mqtt_host}:{mqtt_port}...");
    let (client, eventloop) = AsyncClient::new(mqtt_options, 32);

    state
        .set_hass_client(HassClient {
            client: client.clone(),
        })
        .await;

    let disco_prefix = args.hass_discovery_prefix.clone();
    state.set_hass_disco_prefix(disco_prefix).await;

    tokio::spawn(async move {
        let res = run_mqtt_loop(state, eventloop, client).await;
        if let Err(err) = res {
            log::error!("run_mqtt_loop: {err:#}");
            log::error!("FATAL: hass integration will not function.");
            log::error!("Pausing for 30 seconds before terminating.");
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            std::process::exit(1);
        } else {
            log::info!("run_mqtt_loop exited. We should do something to shutdown gracefully here");
            std::process::exit(0);
        }
    });

    Ok(())
}

pub fn camel_case_to_space_separated(camel: &str) -> String {
    let mut chars = camel.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut result = first.to_ascii_uppercase().to_string();
    for c in chars {
        if c.is_uppercase() {
            result.push(' ');
        }
        result.push(c);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camel_case_to_space_separated() {
        assert_eq!(camel_case_to_space_separated("powerSwitch"), "Power Switch");
        assert_eq!(
            camel_case_to_space_separated("oscillationToggle"),
            "Oscillation Toggle"
        );
    }

    #[test]
    fn test_camel_case_chinese_no_panic() {
        assert_eq!(
            camel_case_to_space_separated("用于三灯头中的第二个"),
            "用于三灯头中的第二个"
        );
    }

    #[test]
    fn test_camel_case_empty() {
        assert_eq!(camel_case_to_space_separated(""), "");
    }

    #[test]
    fn test_camel_case_emoji() {
        assert_eq!(camel_case_to_space_separated("🔥lightMode"), "🔥light Mode");
    }
}
