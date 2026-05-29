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
  /// true when the device is shared into this account rather than owned.
  /// shared devices are controlled via the REST relay (carrying the gas
  /// token) and don't get platform-API state polls.
  shared: boolean;
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
  shared: boolean;
};

// mirrors src/hass_mqtt/base.rs Availability + Device. several of the device
// fields use `#[serde(skip_serializing_if)]` on the rust side and are therefore
// absent from the wire (not present as null) when empty/None; the TS shape
// declares them optional to match what serde produces, not what the type
// theoretically allows.
export type HassAvailability = { topic: string };
export type HassDevice = {
  name: string;
  manufacturer: string;
  model: string;
  sw_version?: string;
  hw_version?: string;
  suggested_area?: string;
  via_device?: string;
  identifiers: string[];
  connections?: [string, string][];
};

// mirrors src/service/state.rs PublishedComponent / PublishedDevice. one
// entry per HA device-discovery config topic, with the device-level
// metadata plus a per-component map of {platform, full config json}.
export type HassPublishedComponent = {
  platform: string;
  config: unknown;
};
export type HassPublishedDevice = {
  device: HassDevice;
  availability: HassAvailability[];
  availability_mode?: "all";
  components: Record<string, HassPublishedComponent>;
};

export type HassPublishedEntry = HassPublishedDevice & { topic: string };

export type HassServiceTopics = {
  availability: string;
  oneclick: string;
  purge_caches: string;
};

export type HassRoute = { pattern: string; purpose: string };

export type HassRegistrationStatus = { at: string };

// /api/debug/hass: the rich debug bundle for the HA integration tab.
// mirrors src/service/http.rs HassDebug.
export type HassDebug = {
  connected: boolean;
  discovery_prefix: string;
  base_topic: string;
  last_registration: HassRegistrationStatus | null;
  service_topics: HassServiceTopics;
  routes: HassRoute[];
  devices: HassPublishedEntry[];
};

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

// /api/recent bundle: the daemon's retained frame ring and every device's
// command history. Fetched on ws connect so a refresh restores the inspector
// and per-device panels without losing what fired before the new socket opens.
export type RecentBundle = {
  frames: Frame[];
  histories: Record<string, CommandLog[]>;
};

// /api/device/{id}/debug bundle.
export type DeviceDebug = {
  device: DeviceItem;
  history: CommandLog[];
};

// mirrors src/service/http.rs DeviceEntity. capability kinds map to the
// platform-API model; the ui renders generic controls per kind. current_value
// is whatever shape the kind reports (number, string, bool, object, array).
//
// the rust DeviceCapabilityKind enum has an Other(String) catch-all variant
// for kinds the daemon hasn't enumerated yet; serde emits it as the bare
// inner string. the `(string & {})` tail keeps the named members visible to
// autocomplete while letting unknown kinds (eg `devices.capabilities.gradient_setting`
// from a future SKU) type-check without falling outside the union.
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
  | "devices.capabilities.event"
  | (string & {});

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
