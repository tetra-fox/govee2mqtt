// mirrors src/service/device.rs DeviceItem / DeviceState and src/service/state.rs
// StateEvent. update both sides if the rust shape changes.

export type Rgb = { r: number; g: number; b: number };

export type DeviceState = {
  on: boolean;
  light_on: boolean | null;
  online: boolean | null;
  kelvin: number;
  color: Rgb;
  brightness: number;
  scene: string | null;
  source: string;
  updated: string;
};

// mirrors src/service/device.rs DeviceCapabilities.
export type DeviceCapabilities = {
  power: boolean;
  brightness: boolean;
  rgb: boolean;
  color_temp_kelvin: [number, number] | null;
  socket_outlets: number | null;
};

export type DeviceItem = {
  sku: string;
  id: string;
  name: string;
  room: string | null;
  ip: string | null;
  state: DeviceState | null;
  capabilities: DeviceCapabilities;
  /// per-outlet on/off for multi-outlet sockets, or null when the daemon
  /// hasn't received an IoT status with the bits yet.
  outlets: boolean[] | null;
};

export type StateEvent =
  | { type: "snapshot"; devices: DeviceItem[] }
  | { type: "device_updated"; device: DeviceItem }
  | { type: "command_logged"; device_id: string; entry: CommandLog }
  | {
      type: "frame";
      device_id: string;
      direction: FrameDirection;
      transport: FrameTransport;
      ts: string;
      payload: string;
    };

export type FrameDirection = "out" | "in";
export type FrameTransport = "ble" | "iot";

// client-side shape; the ws event without the type tag, with ts and
// transport kept for rendering.
export type Frame = {
  device_id: string;
  direction: FrameDirection;
  transport: FrameTransport;
  ts: string;
  payload: string;
};

export type OneClick = {
  // shape comes from /api/oneclicks. exact fields aren't documented yet, so
  // keep this loose and read what's there in the component.
  name: string;
  [key: string]: unknown;
};

// mirrors src/service/http.rs DiscoveryItem. shows which info sources have
// populated and when, plus the quirk-declared transport flags.
export type InfoSources = {
  lan_device: boolean;
  lan_status: boolean;
  http_info: boolean;
  http_state: boolean;
  undoc_info: boolean;
  iot_status: boolean;
};

export type LastSeen = {
  lan_device: string | null;
  lan_status: string | null;
  http_info: string | null;
  http_state: string | null;
  undoc_info: string | null;
  iot_status: string | null;
};

export type DiscoveryItem = {
  sku: string;
  id: string;
  name: string;
  room: string | null;
  ip: string | null;
  ble_address: string | null;
  device_type: string;
  quirk: string | null;
  info_sources: InfoSources;
  effective_transports: Transport[];
  last_seen: LastSeen;
  last_polled: string | null;
};

// /api/debug/hass: HashMap<config_topic, HashMap<component_uid, platform>>.
// outer key is the MQTT discovery config topic, inner key is the component's
// unique id, value is the platform name (light/switch/sensor/etc).
export type HassRegistration = Record<string, Record<string, string>>;

// mirrors src/service/state.rs Transport. snake_case on the wire matches the
// rust #[serde(rename_all = "snake_case")] derive.
export type Transport = "lan" | "ble" | "iot" | "platform" | "iot_nightlight" | "iot_socket";

// mirrors src/service/state.rs CommandOutcome.
export type CommandOutcome =
  | { kind: "ok"; transport: Transport }
  | { kind: "err"; message: string };

// mirrors src/service/state.rs CommandLog. verb + args are structured so the
// ui picks the display format; the daemon doesn't pre-format for us.
export type CommandLog = {
  verb: string;
  args: unknown[];
  started: string;
  finished: string;
  outcome: CommandOutcome;
};

// /api/device/{id}/debug bundle.
export type DeviceDebug = {
  device: DeviceItem;
  history: CommandLog[];
};

// mirrors src/service/http.rs DeviceEntity. capability kinds map to the
// platform-API model; the ui renders generic controls per kind. current_value
// is whatever shape the kind reports (number, string, bool, object, array).
export type DeviceEntityKind =
  | "devices.capabilities.on_off"
  | "devices.capabilities.toggle"
  | "devices.capabilities.range"
  | "devices.capabilities.mode"
  | "devices.capabilities.color_setting"
  | "devices.capabilities.segment_color_setting"
  | "devices.capabilities.music_setting"
  | "devices.capabilities.dynamic_scene"
  | "devices.capabilities.work_mode"
  | "devices.capabilities.dynamic_setting"
  | "devices.capabilities.temperature_setting"
  | "devices.capabilities.online"
  | "devices.capabilities.property"
  | "devices.capabilities.event";

export type DeviceEntityParameters =
  | { dataType: "ENUM"; options: { name: string; value: unknown }[] }
  | {
      dataType: "INTEGER";
      unit: string | null;
      range: { min: number; max: number; precision: number };
    }
  | { dataType: "STRUCT"; fields: unknown[] }
  | {
      dataType: "Array";
      size: unknown;
      elementRange: unknown;
      elementType: string | null;
      options: unknown[];
    };

export type DeviceEntity = {
  instance: string;
  name: string;
  kind: DeviceEntityKind;
  parameters: DeviceEntityParameters | null;
  current_value: unknown;
};

// mirrors src/service/http.rs DebugInfo, which flattens ServiceInfo plus a
// live ClientsStatus snapshot. sensitive values come down as plain strings;
// the ui decides which fields to mask (see InfoView.svelte).
export type DebugInfo = {
  version: string;
  http_port: number;
  availability_timeout_secs: number;
  ble_enabled: boolean;
  govee: {
    platform_endpoint: string;
    undoc_endpoint: string;
    api_key: string | null;
    email: string | null;
    password: string | null;
    amazon_root_ca: string;
  };
  mqtt: {
    host: string | null;
    port: number;
    username: string | null;
    password: string | null;
    base_topic: string;
  };
  hass: {
    discovery_prefix: string;
    temperature_scale: string;
  };
  clients: {
    lan: boolean;
    ble: boolean;
    iot: boolean;
    platform: boolean;
    undoc: boolean;
    hass: boolean;
  };
  devices: number;
};
