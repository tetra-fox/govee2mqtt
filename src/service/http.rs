use crate::service::coordinator::Coordinator;
use crate::service::device::{Device, DeviceItem};
use crate::service::info::ServiceInfo;
use crate::service::state::{CommandLog, StateEvent, StateHandle, Transport};
use anyhow::Context;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use govee_api::model::{DeviceCapability, DeviceCapabilityKind, DeviceParameters};
use govee_api::platform_api::DeviceType;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::net::IpAddr;
use tokio::sync::broadcast::error::RecvError;

/// Static UI assets. The ui/ subdirectory is built with `pnpm build` into
/// ui/dist/; rust-embed bakes that into the binary at compile time. With the
/// `debug-embed` feature the assets are also embedded in debug builds, so
/// `cargo run` serves whatever was built most recently rather than reading
/// from disk.
#[derive(rust_embed::Embed)]
#[folder = "ui/dist/"]
struct UiAssets;

fn response_with_code<T: ToString + std::fmt::Display>(code: StatusCode, err: T) -> Response {
    if !code.is_success() {
        log::error!("err: {err:#}");
    }

    let mut response = Json(serde_json::json!({
        "code": code.as_u16(),
        "msg": format!("{err:#}")
    }))
    .into_response();
    *response.status_mut() = code;
    response
}

fn generic<T: ToString + std::fmt::Display>(err: T) -> Response {
    response_with_code(StatusCode::INTERNAL_SERVER_ERROR, err)
}

fn not_found<T: ToString + std::fmt::Display>(err: T) -> Response {
    response_with_code(StatusCode::NOT_FOUND, err)
}

fn bad_request<T: ToString + std::fmt::Display>(err: T) -> Response {
    response_with_code(StatusCode::BAD_REQUEST, err)
}

async fn resolve_device_for_control(
    state: &StateHandle,
    id: &str,
) -> Result<Coordinator, Response> {
    state
        .resolve_device_for_control(id)
        .await
        .map_err(not_found)
}

async fn resolve_device_read_only(state: &StateHandle, id: &str) -> Result<Device, Response> {
    state.resolve_device_read_only(id).await.map_err(not_found)
}

/// Returns a json array of device information
async fn list_devices(State(state): State<StateHandle>) -> Result<Response, Response> {
    Ok(Json(snapshot_all(&state).await).into_response())
}

/// Sorted projection of the current device set, used by /api/devices and as
/// the StateEvent::Snapshot payload on ws connect and lag-resync.
async fn snapshot_all(state: &StateHandle) -> Vec<DeviceItem> {
    let mut devices = state.devices().await;
    devices.sort_by_key(|d| (d.room_name().map(|name| name.to_string()), d.name()));
    devices.iter().map(DeviceItem::snapshot).collect()
}

/// Turns on a given device
async fn device_power_on(
    State(state): State<StateHandle>,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let device = resolve_device_for_control(&state, &id).await?;

    state
        .device_power_on(&device, true)
        .await
        .map_err(generic)?;

    Ok(response_with_code(StatusCode::OK, "ok"))
}

/// Switch a single outlet of a multi-outlet socket (eg H5082). `index` is
/// the zero-based outlet number per the daemon's socket_outlet_count.
async fn device_outlet_on(
    State(state): State<StateHandle>,
    Path((id, index)): Path<(String, u8)>,
) -> Result<Response, Response> {
    let device = resolve_device_for_control(&state, &id).await?;
    state
        .device_set_socket_outlet(&device, index, true)
        .await
        .map_err(generic)?;
    Ok(response_with_code(StatusCode::OK, "ok"))
}

async fn device_outlet_off(
    State(state): State<StateHandle>,
    Path((id, index)): Path<(String, u8)>,
) -> Result<Response, Response> {
    let device = resolve_device_for_control(&state, &id).await?;
    state
        .device_set_socket_outlet(&device, index, false)
        .await
        .map_err(generic)?;
    Ok(response_with_code(StatusCode::OK, "ok"))
}

/// Turns off a given device
async fn device_power_off(
    State(state): State<StateHandle>,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let device = resolve_device_for_control(&state, &id).await?;

    state
        .device_power_on(&device, false)
        .await
        .map_err(generic)?;

    Ok(response_with_code(StatusCode::OK, "ok"))
}

/// Sets the brightness level of a given device
async fn device_set_brightness(
    State(state): State<StateHandle>,
    Path((id, level)): Path<(String, u8)>,
) -> Result<Response, Response> {
    let device = resolve_device_for_control(&state, &id).await?;

    state
        .device_set_brightness(&device, level)
        .await
        .map_err(generic)?;

    Ok(response_with_code(StatusCode::OK, "ok"))
}

/// Sets the color temperature of a given device
async fn device_set_color_temperature(
    State(state): State<StateHandle>,
    Path((id, kelvin)): Path<(String, u32)>,
) -> Result<Response, Response> {
    let device = resolve_device_for_control(&state, &id).await?;

    state
        .device_set_color_temperature(&device, kelvin)
        .await
        .map_err(generic)?;

    Ok(response_with_code(StatusCode::OK, "ok"))
}

/// Sets the RGB color of a given device
async fn device_set_color(
    State(state): State<StateHandle>,
    Path((id, color)): Path<(String, String)>,
) -> Result<Response, Response> {
    let color = csscolorparser::parse(&color)
        .map_err(|err| bad_request(format!("error parsing color '{color}': {err}")))?;
    let [r, g, b, _a] = color.to_rgba8();

    let device = resolve_device_for_control(&state, &id).await?;

    state
        .device_set_color_rgb(&device, r, g, b)
        .await
        .map_err(generic)?;

    Ok(response_with_code(StatusCode::OK, "ok"))
}

/// Activates the named scene for a given device
async fn device_set_scene(
    State(state): State<StateHandle>,
    Path((id, scene)): Path<(String, String)>,
) -> Result<Response, Response> {
    let device = resolve_device_for_control(&state, &id).await?;

    state
        .device_set_scene(&device, &scene)
        .await
        .map_err(generic)?;

    Ok(response_with_code(StatusCode::OK, "ok"))
}

/// Returns a JSON array of the available scene names for a given device
async fn device_list_scenes(
    State(state): State<StateHandle>,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let device = resolve_device_read_only(&state, &id).await?;

    let scenes = state.device_list_scenes(&device).await.map_err(generic)?;

    Ok(Json(scenes).into_response())
}

async fn list_one_clicks(State(state): State<StateHandle>) -> Result<Response, Response> {
    let undoc = state
        .get_undoc_client()
        .await
        .ok_or_else(|| anyhow::anyhow!("Undoc API client is not available"))
        .map_err(generic)?;
    let items = undoc.parse_one_clicks().await.map_err(generic)?;

    Ok(Json(items).into_response())
}

async fn activate_one_click(
    State(state): State<StateHandle>,
    Path(name): Path<String>,
) -> Result<Response, Response> {
    let undoc = state
        .get_undoc_client()
        .await
        .ok_or_else(|| anyhow::anyhow!("Undoc API client is not available"))
        .map_err(generic)?;
    let items = undoc.parse_one_clicks().await.map_err(generic)?;
    let item = items
        .iter()
        .find(|item| item.name == name)
        .ok_or_else(|| anyhow::anyhow!("didn't find item {name}"))
        .map_err(not_found)?;

    let iot = state
        .get_iot_client()
        .await
        .ok_or_else(|| anyhow::anyhow!("AWS IoT client is not available"))
        .map_err(generic)?;

    iot.activate_one_click(item).await.map_err(generic)?;

    Ok(response_with_code(StatusCode::OK, "ok"))
}

/// Per-device entry returned by /api/debug/discovery. Surfaces which info
/// sources have populated, which transports the quirk says are reachable,
/// and the timestamps of the last update from each source. None of this is
/// derived from the wire DeviceItem: it answers "what does the daemon think
/// it knows about this device, and how did it learn each fact."
#[derive(Serialize)]
struct DiscoveryItem {
    sku: String,
    id: String,
    name: String,
    room: Option<String>,
    ip: Option<IpAddr>,
    ble_address: Option<String>,
    device_type: String,
    /// SKU of the matched static quirk, if any. None means the device is being
    /// handled via generic platform-API capabilities only.
    quirk: Option<String>,
    info_sources: InfoSources,
    /// Transports the cascade could actually reach for this device right now.
    /// Combines the controller-type-specific path (eg sockets always route
    /// through `IotSocket`) with the generic cascade's runtime eligibility
    /// (LAN if a LAN device was discovered, BLE if an adapter is loaded and
    /// the device has a BLE address, IoT if owned and an IoT client is up,
    /// platform if an http_device_info exists and the quirk doesn't opt out).
    effective_transports: Vec<Transport>,
    last_seen: LastSeen,
    last_polled: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct InfoSources {
    lan_device: bool,
    lan_status: bool,
    http_info: bool,
    http_state: bool,
    undoc_info: bool,
    iot_status: bool,
}

#[derive(Serialize)]
struct LastSeen {
    lan_device: Option<DateTime<Utc>>,
    lan_status: Option<DateTime<Utc>>,
    http_info: Option<DateTime<Utc>>,
    http_state: Option<DateTime<Utc>>,
    undoc_info: Option<DateTime<Utc>>,
    iot_status: Option<DateTime<Utc>>,
}

/// Snapshot of runtime client availability used to compute effective transports.
/// Sampled once per /api/debug/discovery call so the cascade evaluation matches
/// what `transport.rs` would actually find at that moment.
struct ClientAvail {
    lan: bool,
    ble: bool,
    iot: bool,
    platform: bool,
}

impl DiscoveryItem {
    fn from_device(d: &Device, avail: &ClientAvail) -> Self {
        let quirk = d.resolve_quirk();
        Self {
            sku: d.sku.clone(),
            id: d.id.clone(),
            name: d.name(),
            room: d.room_name().map(|r| r.to_string()),
            ip: d.ip_addr(),
            ble_address: d.ble_address().map(|s| s.to_string()),
            device_type: d.device_type().to_string(),
            quirk: quirk.as_ref().map(|q| q.sku.to_string()),
            info_sources: InfoSources {
                lan_device: d.lan_device.is_some(),
                lan_status: d.lan_device_status.is_some(),
                http_info: d.http_device_info.is_some(),
                http_state: d.http_device_state.is_some(),
                undoc_info: d.undoc_device_info.is_some(),
                iot_status: d.iot_device_status.is_some(),
            },
            effective_transports: effective_transports(d, avail),
            last_seen: LastSeen {
                lan_device: d.last_lan_device_update,
                lan_status: d.last_lan_device_status_update,
                http_info: d.last_http_device_update,
                http_state: d.last_http_device_state_update,
                undoc_info: d.last_undoc_device_info_update,
                iot_status: d.last_iot_device_status_update,
            },
            last_polled: d.last_polled,
        }
    }
}

/// Mirror the controller and cascade logic from src/service/{control,transport}.rs
/// to decide which transports could actually accept a command for this device
/// right now. Read-only: no calls into the cascade, just predicates on the
/// device's facts and the runtime client availability snapshot.
fn effective_transports(d: &Device, avail: &ClientAvail) -> Vec<Transport> {
    match d.device_type() {
        // SocketController short-circuits the cascade and always takes the
        // socket_turn IoT path.
        DeviceType::Socket => vec![Transport::IotSocket],
        // HumidifierController tries the nightlight packet first, then falls
        // through to the generic cascade for the rest.
        DeviceType::Humidifier | DeviceType::Dehumidifier => {
            let mut out = vec![Transport::IotNightlight];
            out.extend(generic_cascade(d, avail));
            out
        }
        _ => generic_cascade(d, avail),
    }
}

/// The order the generic cascade would try: LAN, BLE, IoT, platform. Each is
/// included only when the underlying conditions are met.
fn generic_cascade(d: &Device, avail: &ClientAvail) -> Vec<Transport> {
    let mut out = Vec::new();
    if avail.lan && d.lan_device.is_some() {
        out.push(Transport::Lan);
    }
    if avail.ble && d.ble_address().is_some() {
        out.push(Transport::Ble);
    }
    if avail.iot && d.iot_api_supported() && d.undoc_device_info.is_some() {
        out.push(Transport::Iot);
    }
    if avail.platform && d.http_device_info.is_some() && !d.avoid_platform_api() {
        out.push(Transport::Platform);
    }
    out
}

async fn list_discovery(State(state): State<StateHandle>) -> Response {
    let avail = ClientAvail {
        lan: state.get_lan_client().await.is_some(),
        ble: state.get_ble_client().await.is_some(),
        iot: state.get_iot_client().await.is_some(),
        platform: state.get_platform_client().await.is_some(),
    };
    let mut devices = state.devices().await;
    devices.sort_by_key(|d| (d.room_name().map(|n| n.to_string()), d.name()));
    let items: Vec<DiscoveryItem> = devices
        .iter()
        .map(|d| DiscoveryItem::from_device(d, &avail))
        .collect();
    Json(items).into_response()
}

async fn list_hass_registration(State(state): State<StateHandle>) -> Response {
    let components = state.get_published_components().await;
    Json(components).into_response()
}

/// Bundle returned by /api/debug/info. Pairs the static configuration captured
/// at serve startup with a live snapshot of which transport clients are
/// connected so the UI can show "configured" alongside "actually up".
#[derive(Serialize)]
struct DebugInfo {
    #[serde(flatten)]
    service: ServiceInfo,
    clients: ClientsStatus,
    devices: usize,
}

#[derive(Serialize)]
struct ClientsStatus {
    lan: bool,
    ble: bool,
    iot: bool,
    platform: bool,
    undoc: bool,
    hass: bool,
}

/// One controllable/observable entity surfaced by the daemon. The list parallels
/// what hass_mqtt::enumerator publishes to HA, but as a flat capability-shaped
/// view the web UI can render with generic per-kind controls. The shape mirrors
/// the platform-API DeviceCapability so consumers already familiar with that
/// schema don't need new vocabulary; `current_value` is the live state of the
/// instance per `get_state_capability_by_instance`, or null when no value has
/// been observed yet (eg pre-discovery state).
#[derive(Serialize)]
struct DeviceEntity {
    instance: String,
    /// Curated friendly label per ble::entity_name with a camelCase fallback;
    /// matches the HA entity name shown in the Home Assistant ui.
    name: String,
    kind: DeviceCapabilityKind,
    parameters: Option<DeviceParameters>,
    current_value: Option<JsonValue>,
}

impl DeviceEntity {
    fn from_capability(device: &Device, cap: &DeviceCapability) -> Self {
        // Both the platform-API state shape and the synthesized ble::state_value
        // wrap the actual value as `{"value": <v>}` (the platform protocol's
        // convention; ble::projector and ble::socket mirror it so HA's
        // notify_state can read `state["value"]` uniformly). The web UI doesn't
        // care about the protocol convention; unwrap so consumers see the bare
        // value for number/bool/string instances and don't have to dig.
        let current_value = device
            .get_state_capability_by_instance(&cap.instance)
            .map(|s| match s.state {
                JsonValue::Object(mut o) => o.remove("value").unwrap_or(JsonValue::Object(o)),
                other => other,
            });
        Self {
            instance: cap.instance.clone(),
            name: crate::service::hass::entity_display_name(&cap.instance),
            kind: cap.kind.clone(),
            parameters: cap.parameters.clone(),
            current_value,
        }
    }
}

/// List every capability the daemon exposes for this device, with current
/// values. The same set of capabilities that hass_mqtt walks to register HA
/// entities; this endpoint just hands them back in a generic shape so the web
/// ui can render generic controls without hardcoding each capability.
async fn device_entities(
    State(state): State<StateHandle>,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let device = resolve_device_read_only(&state, &id).await?;
    let caps = device
        .http_device_info
        .as_ref()
        .map(|info| info.capabilities.as_slice())
        .unwrap_or(&[]);
    let entities: Vec<DeviceEntity> = caps
        .iter()
        .map(|cap| DeviceEntity::from_capability(&device, cap))
        .collect();
    Ok(Json(entities).into_response())
}

/// Body for the generic capability-set endpoint. Just the JSON value to write
/// for the instance; kind is implied by the device's capability metadata.
#[derive(Deserialize)]
struct CapabilityBody {
    value: JsonValue,
}

/// Set a single capability's value by instance. Routes through `device_control`
/// (which goes through the chokepoint and lands in command history). The body
/// is `{"value": <any json>}` — daemon doesn't try to validate the value
/// against the capability's parameters; the caller is expected to send a value
/// compatible with the capability's kind. Errors propagate from the cascade.
async fn device_set_capability(
    State(state): State<StateHandle>,
    Path((id, instance)): Path<(String, String)>,
    Json(body): Json<CapabilityBody>,
) -> Result<Response, Response> {
    let device = resolve_device_for_control(&state, &id).await?;
    let cap = device
        .get_capability_by_instance(&instance)
        .cloned()
        .ok_or_else(|| not_found(format!("unknown capability instance {instance}")))?;
    state
        .device_control(&device, &cap, body.value)
        .await
        .map_err(generic)?;
    Ok(response_with_code(StatusCode::OK, "ok"))
}

async fn debug_info(State(state): State<StateHandle>) -> Result<Response, Response> {
    let service = state
        .get_service_info()
        .await
        .ok_or_else(|| generic("service info is not available"))?;
    let clients = ClientsStatus {
        lan: state.get_lan_client().await.is_some(),
        ble: state.get_ble_client().await.is_some(),
        iot: state.get_iot_client().await.is_some(),
        platform: state.get_platform_client().await.is_some(),
        undoc: state.get_undoc_client().await.is_some(),
        hass: state.get_hass_client().await.is_some(),
    };
    let devices = state.devices().await.len();
    Ok(Json(DebugInfo {
        service,
        clients,
        devices,
    })
    .into_response())
}

/// Per-device debug bundle: the wire DeviceItem plus the daemon's command
/// history ring for the device. The UI detail panel renders these together
/// so the user sees current state and the trail of commands that produced it.
#[derive(Serialize)]
struct DeviceDebug {
    device: DeviceItem,
    history: Vec<CommandLog>,
}

async fn device_debug(
    State(state): State<StateHandle>,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let device = resolve_device_read_only(&state, &id).await?;
    let bundle = DeviceDebug {
        device: DeviceItem::snapshot(&device),
        history: state.get_command_history(&device.id).await,
    };
    Ok(Json(bundle).into_response())
}

/// Force a fresh poll of a device. Best-effort across IoT and platform; LAN
/// devices push their own state and don't need polling. Returns OK as soon
/// as one path completed without error (or both failed).
async fn device_force_poll(
    State(state): State<StateHandle>,
    Path(id): Path<String>,
) -> Result<Response, Response> {
    let device = resolve_device_read_only(&state, &id).await?;
    let iot_ok = state.poll_iot_api(&device).await.is_ok();
    let platform_ok = state.poll_platform_api(&device).await.is_ok();
    if iot_ok || platform_ok {
        Ok(response_with_code(StatusCode::OK, "ok"))
    } else {
        Err(generic("no transport could poll this device"))
    }
}

/// Upgrade handler for /ws. Subscribe before sending the initial snapshot so
/// no state change that lands between snapshot and subscribe is lost.
async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<StateHandle>) -> Response {
    ws.on_upgrade(|socket| ws_session(socket, state))
}

async fn ws_session(mut socket: WebSocket, state: StateHandle) {
    let mut rx = state.subscribe();

    let initial = StateEvent::Snapshot {
        devices: snapshot_all(&state).await,
    };
    if send_event(&mut socket, &initial).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Ok(ev) => {
                    if send_event(&mut socket, &ev).await.is_err() {
                        return;
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    // dropped frames past the ring capacity; resend a full
                    // snapshot so the client's view is consistent again
                    let resync = StateEvent::Snapshot {
                        devices: snapshot_all(&state).await,
                    };
                    if send_event(&mut socket, &resync).await.is_err() {
                        return;
                    }
                }
                Err(RecvError::Closed) => return,
            },
            // pump inbound so close and ping frames are handled; commands go
            // over rest, so any client text is ignored for now
            msg = socket.recv() => match msg {
                Some(Ok(_)) => {}
                _ => return,
            },
        }
    }
}

async fn send_event(socket: &mut WebSocket, ev: &StateEvent) -> Result<(), ()> {
    let text = serde_json::to_string(ev).map_err(|_| ())?;
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| ())
}

/// Serve an embedded UI asset by path. Falls back to index.html for unknown
/// paths so client-side router deep links work; api and ws routes take
/// precedence because the fallback handler only fires when no route matched.
async fn ui_handler(uri: Uri) -> Response {
    // strip leading '/' for the rust-embed lookup
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    let asset = UiAssets::get(path).or_else(|| UiAssets::get("index.html"));
    let Some(asset) = asset else {
        // ui/dist was empty at build time. happens when running without first
        // building the ui; tell the user what to do instead of 404.
        return response_with_code(
            StatusCode::NOT_FOUND,
            "ui assets are not embedded; build the ui with `cd ui && pnpm build`",
        );
    };

    // pick the right content-type from the requested path's extension, not
    // index.html's, so a missing .js still serves as text/html (rare) and a
    // valid path serves with its real type.
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut resp = asset.data.into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        mime.as_ref().parse().expect("valid mime"),
    );
    resp
}

fn build_router(state: StateHandle) -> Router {
    Router::new()
        .route("/api/devices", get(list_devices))
        .route("/api/device/{id}/power/on", get(device_power_on))
        .route("/api/device/{id}/power/off", get(device_power_off))
        .route("/api/device/{id}/outlet/{index}/on", get(device_outlet_on))
        .route(
            "/api/device/{id}/outlet/{index}/off",
            get(device_outlet_off),
        )
        .route(
            "/api/device/{id}/brightness/{level}",
            get(device_set_brightness),
        )
        .route(
            "/api/device/{id}/colortemp/{kelvin}",
            get(device_set_color_temperature),
        )
        .route("/api/device/{id}/color/{color}", get(device_set_color))
        .route("/api/device/{id}/scene/{scene}", get(device_set_scene))
        .route("/api/device/{id}/scenes", get(device_list_scenes))
        .route("/api/device/{id}/entities", get(device_entities))
        .route(
            "/api/device/{id}/capability/{instance}",
            post(device_set_capability),
        )
        .route("/api/oneclicks", get(list_one_clicks))
        .route("/api/oneclick/activate/{scene}", get(activate_one_click))
        .route("/api/debug/discovery", get(list_discovery))
        .route("/api/debug/hass", get(list_hass_registration))
        .route("/api/debug/info", get(debug_info))
        .route("/api/device/{id}/debug", get(device_debug))
        .route("/api/device/{id}/poll", get(device_force_poll))
        .route("/ws", get(ws_upgrade))
        .fallback(ui_handler)
        .with_state(state)
}

#[cfg(test)]
#[test]
fn test_build_router() {
    // axum has a history of chaning the URL syntax across
    // semver bumps; while that is OK, the syntax changes
    // are not caught at compile time, so we need a runtime
    // check to verify that the syntax is still good.
    // This next line will panic if axum decides that
    // the syntax is bad.
    let _ = build_router(StateHandle::default());
}

pub async fn run_http_server(state: StateHandle, port: u16) -> anyhow::Result<()> {
    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .with_context(|| format!("run_http_server: binding to port {port}"))?;
    let addr = listener.local_addr()?;
    log::info!("HTTP server listening on {addr}");
    if let Err(err) = axum::serve(listener, app).await {
        log::error!("HTTP server stopped: {err:#}");
    }

    Ok(())
}
