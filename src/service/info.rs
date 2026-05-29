use serde::Serialize;

/// Static daemon configuration captured once at serve startup. The runtime
/// portion (which clients are connected, devices count) is filled in at
/// request time and merged on top by the http handler. Sensitive fields are
/// emitted as plain strings; the consumer decides whether and how to mask.
#[derive(Serialize, Clone, Debug)]
pub struct ServiceInfo {
    pub version: String,
    pub http_port: u16,
    pub availability_timeout_secs: i64,
    pub ble_enabled: bool,
    pub govee: GoveeInfo,
    pub mqtt: MqttInfo,
    pub hass: HassInfo,
}

#[derive(Serialize, Clone, Debug)]
pub struct GoveeInfo {
    pub platform_endpoint: String,
    pub undoc_endpoint: String,
    pub api_key: Option<String>,
    pub email: Option<String>,
    pub password: Option<String>,
    pub amazon_root_ca: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct MqttInfo {
    pub host: Option<String>,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub base_topic: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct HassInfo {
    pub discovery_prefix: String,
    pub temperature_scale: String,
}
