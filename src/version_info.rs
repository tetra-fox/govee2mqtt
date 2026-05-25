// GOVEE2MQTT_VERSION is composed by build.rs. It is empty for local dev builds that
// pass neither CI input, in which case the bare Cargo.toml version is used.
const VERSION: &str = env!("GOVEE2MQTT_VERSION");
const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn govee_version() -> &'static str {
    if VERSION.is_empty() {
        PKG_VERSION
    } else {
        VERSION
    }
}
