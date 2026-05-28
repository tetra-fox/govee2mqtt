use govee_api::platform_api::DeviceType;
use govee_api::temperature::TemperatureUnits;
use once_cell::sync::Lazy;
use std::borrow::Cow;
use std::collections::HashMap;

#[allow(unused)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HumidityUnits {
    RelativePercent,
    RelativePercentTimes100,
}

impl HumidityUnits {
    #[allow(clippy::wrong_self_convention)]
    pub fn from_reading_to_relative_percent(&self, value: f64) -> f64 {
        match self {
            Self::RelativePercent => value,
            Self::RelativePercentTimes100 => value / 100.,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Quirk {
    pub sku: Cow<'static, str>,
    pub icon: Cow<'static, str>,
    pub supports_rgb: bool,
    pub supports_brightness: bool,
    pub color_temp_range: Option<(u32, u32)>,
    pub avoid_platform_api: bool,
    pub ble_only: bool,
    pub lan_api_capable: bool,
    pub device_type: DeviceType,
    pub platform_temperature_sensor_units: Option<TemperatureUnits>,
    pub platform_humidity_sensor_units: Option<HumidityUnits>,
    /// If true, we can correctly parse all appropriate
    /// packets from the MQTT subscription and apply
    /// their state.
    pub iot_api_supported: bool,
    pub show_as_preset_buttons: Option<&'static [&'static str]>,
    /// For sockets that expose more than one independently switched
    /// outlet but only report a single combined powerSwitch via the
    /// platform API. The IoT status packet packs each outlet into one
    /// bit of the onOff value, so we can at least report the per-outlet
    /// state. See <https://github.com/wez/govee2mqtt/issues/65>
    pub socket_outlet_count: Option<u8>,
}

impl Quirk {
    pub fn device<SKU: Into<Cow<'static, str>>>(
        sku: SKU,
        device_type: DeviceType,
        icon: &'static str,
    ) -> Self {
        Self {
            sku: sku.into(),
            supports_rgb: false,
            supports_brightness: false,
            color_temp_range: None,
            avoid_platform_api: false,
            ble_only: false,
            icon: icon.into(),
            lan_api_capable: false,
            device_type,
            platform_temperature_sensor_units: None,
            platform_humidity_sensor_units: None,
            iot_api_supported: false,
            show_as_preset_buttons: None,
            socket_outlet_count: None,
        }
    }

    pub fn light<SKU: Into<Cow<'static, str>>>(sku: SKU, icon: &'static str) -> Self {
        Self::device(sku, DeviceType::Light, icon)
            .with_rgb()
            .with_brightness()
            .with_color_temp()
            .with_iot_api_support(true)
    }

    pub fn ice_maker<SKU: Into<Cow<'static, str>>>(sku: SKU) -> Self {
        Self::device(sku, DeviceType::IceMaker, "mdi:snowflake")
    }

    pub fn space_heater<SKU: Into<Cow<'static, str>>>(sku: SKU) -> Self {
        Self::device(sku, DeviceType::Heater, "mdi:heat-wave")
    }

    pub fn humidifier<SKU: Into<Cow<'static, str>>>(sku: SKU) -> Self {
        Self::device(sku, DeviceType::Humidifier, "mdi:air-humidifier")
    }

    pub fn thermometer<SKU: Into<Cow<'static, str>>>(sku: SKU) -> Self {
        Self::device(sku, DeviceType::Thermometer, "mdi:thermometer")
    }

    pub fn with_rgb(mut self) -> Self {
        self.supports_rgb = true;
        self
    }

    pub fn with_brightness(mut self) -> Self {
        self.supports_brightness = true;
        self
    }

    pub fn with_platform_temperature_sensor_units(mut self, units: TemperatureUnits) -> Self {
        self.platform_temperature_sensor_units = Some(units);
        self
    }

    pub fn with_platform_humidity_sensor_units(mut self, units: HumidityUnits) -> Self {
        self.platform_humidity_sensor_units = Some(units);
        self
    }

    pub fn with_iot_api_support(mut self, supported: bool) -> Self {
        self.iot_api_supported = supported;
        self
    }

    pub fn with_color_temp(mut self) -> Self {
        self.color_temp_range = Some((2000, 9000));
        self
    }

    pub fn with_color_temp_range(mut self, min: u32, max: u32) -> Self {
        self.color_temp_range = Some((min, max));
        self
    }

    pub fn with_lan_api(mut self) -> Self {
        self.lan_api_capable = true;
        self
    }

    pub fn with_show_as_preset_modes(mut self, modes: &'static [&'static str]) -> Self {
        self.show_as_preset_buttons.replace(modes);
        self
    }

    pub fn with_socket_outlets(mut self, count: u8) -> Self {
        self.socket_outlet_count.replace(count);
        self
    }

    pub fn with_broken_platform(mut self) -> Self {
        self.avoid_platform_api = true;
        self
    }

    pub fn with_ble_only(mut self, ble_only: bool) -> Self {
        self.ble_only = ble_only;
        self
    }

    pub fn lan_api_capable_light(sku: &'static str, icon: &'static str) -> Self {
        Self::light(sku, icon).with_lan_api()
    }

    pub fn should_show_mode_as_preset(&self, mode: &str) -> bool {
        self.show_as_preset_buttons
            .as_ref()
            .map(|modes| modes.contains(&mode))
            .unwrap_or(false)
    }
}

static QUIRKS: Lazy<HashMap<String, Quirk>> = Lazy::new(load_quirks);

const STRIP: &str = "mdi:led-strip-variant";
const STRIP_ALT: &str = "mdi:led-strip";
const FLOOD: &str = "mdi:light-flood-down";
const STRING: &str = "mdi:string-lights";
pub const BULB: &str = "mdi:lightbulb";
const FLOOR_LAMP: &str = "mdi:floor-lamp";
const TV_BACK: &str = "mdi:television-ambient-light";
const DESK: &str = "mdi:desk-lamp";
const HEX: &str = "mdi:hexagon-multiple";
const TRIANGLE: &str = "mdi:triangle";
const CEILING: &str = "mdi:ceiling-light";
const NIGHTLIGHT: &str = "mdi:lightbulb-night";
const WALL_SCONCE: &str = "mdi:wall-sconce";
const OUTDOOR_LAMP: &str = "mdi:outdoor-lamp";
const SPOTLIGHT: &str = "mdi:lightbulb-spot";
const POWER_SOCKET: &str = "mdi:power-socket";
const PROJECTOR: &str = "mdi:projector";
const CEILING_FAN: &str = "mdi:ceiling-fan-light";

fn load_quirks() -> HashMap<String, Quirk> {
    let mut map = HashMap::new();
    for quirk in [
        // H60A1 Govee Ceiling Light has a color temperature range of 2200K - 6500K
        // Without this quirk, the LAN API fallback reports (2000, 9000) which causes issues
        // <https://github.com/wez/govee2mqtt/pull/502>
        Quirk::lan_api_capable_light("H60A1", CEILING).with_color_temp_range(2200, 6500),
        // Color temperature is more restrictive than the fallback range
        // <https://github.com/wez/govee2mqtt/issues/511>
        Quirk::lan_api_capable_light("H6022", BULB).with_color_temp_range(2700, 6500),
        Quirk::lan_api_capable_light("H610A", STRIP),
        // At the time of writing, the metadata
        // returned by Govee is completely bogus for this
        // device
        // <https://github.com/wez/govee2mqtt/issues/15>
        Quirk::light("H6141", STRIP).with_broken_platform(),
        // At the time of writing, the metadata
        // returned by Govee is completely bogus for this
        // device
        // <https://github.com/wez/govee2mqtt/issues/14#issuecomment-1880050091>
        Quirk::light("H6159", STRIP).with_broken_platform(),
        // <https://github.com/wez/govee2mqtt/issues/152>
        Quirk::light("H6003", BULB).with_broken_platform(),
        // <https://github.com/wez/govee2mqtt/issues/40#issuecomment-1889726710>
        // indicates that this one doesn't work like the others with IoT
        Quirk::light("H6121", STRIP).with_iot_api_support(false),
        // <https://github.com/wez/govee2mqtt/issues/40>
        Quirk::light("H6154", STRIP).with_iot_api_support(false),
        // <https://github.com/wez/govee2mqtt/issues/49>
        Quirk::light("H6176", STRIP).with_iot_api_support(false),
        // Platform API probably shouldn't return this device (I suppose,
        // aside from letting us find out its name), and we need to know
        // that it is definitely BLE-only
        // <https://github.com/wez/govee2mqtt/issues/92>
        Quirk::light("H6102", STRIP)
            .with_broken_platform()
            .with_ble_only(true),
        // Another BLE-only device <https://github.com/wez/govee2mqtt/issues/77>
        Quirk::light("H6053", STRIP)
            .with_broken_platform()
            .with_ble_only(true),
        Quirk::light("H617C", STRIP)
            .with_broken_platform()
            .with_ble_only(true),
        Quirk::light("H617E", STRIP)
            .with_broken_platform()
            .with_ble_only(true),
        Quirk::light("H617F", STRIP)
            .with_broken_platform()
            .with_ble_only(true),
        Quirk::light("H6119", STRIP)
            .with_broken_platform()
            .with_ble_only(true),
        // Humidifer with mangled platform API data
        Quirk::humidifier("H7160")
            .with_broken_platform()
            .with_iot_api_support(true)
            .with_rgb()
            .with_brightness(),
        // Additional humidifiers grouped with H7160 in homebridge-govee's
        // `humidifier` bucket (lib/utils/constants.js). Added without
        // with_broken_platform because we have not verified that the platform
        // API misreports them like it does H7160; flip the flag per-SKU if a
        // user reports the same mangled-metadata issue. RGB+brightness left
        // off for the same reason (not every humidifier has the nightlight).
        Quirk::humidifier("H7140").with_iot_api_support(true),
        Quirk::humidifier("H7141").with_iot_api_support(true),
        Quirk::humidifier("H7142").with_iot_api_support(true),
        Quirk::humidifier("H7143").with_iot_api_support(true),
        Quirk::humidifier("H7145").with_iot_api_support(true),
        Quirk::humidifier("H7147").with_iot_api_support(true),
        Quirk::humidifier("H7148").with_iot_api_support(true),
        Quirk::humidifier("H7149").with_iot_api_support(true),
        Quirk::humidifier("H714E").with_iot_api_support(true),
        // Dehumidifiers grouped under homebridge-govee's `dehumidifier` bucket.
        // We do not have any dehumidifier quirks yet; the Dehumidifier device
        // type already routes through the humidifier controller and HA entity,
        // so registering these enables basic on/off + mode + humidity reading.
        Quirk::device("H7150", DeviceType::Dehumidifier, "mdi:air-humidifier-off")
            .with_iot_api_support(true),
        Quirk::device("H7151", DeviceType::Dehumidifier, "mdi:air-humidifier-off")
            .with_iot_api_support(true),
        Quirk::device("H7152", DeviceType::Dehumidifier, "mdi:air-humidifier-off")
            .with_iot_api_support(true),
        Quirk::space_heater("H7130")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit),
        Quirk::space_heater("H7131")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit)
            .with_show_as_preset_modes(&["gearMode"])
            .with_rgb()
            .with_brightness(),
        Quirk::space_heater("H713A")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit),
        Quirk::space_heater("H713B")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit),
        // H713C is grouped with H713A/H713B in homebridge-govee's `heater1` bucket
        // (lib/utils/constants.js); same shape applies.
        Quirk::space_heater("H713C")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit),
        Quirk::space_heater("H7132")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit),
        Quirk::space_heater("H7133")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit)
            .with_show_as_preset_modes(&["gearMode"])
            .with_rgb()
            .with_brightness(),
        Quirk::space_heater("H7134")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit)
            .with_show_as_preset_modes(&["gearMode"])
            .with_color_temp()
            .with_brightness(),
        Quirk::space_heater("H7135")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit),
        // <https://github.com/wez/govee2mqtt/issues/343>
        Quirk::ice_maker("H7172").with_iot_api_support(false),
        // Additional ice makers grouped with H7172 in homebridge-govee's
        // `iceMaker` bucket (lib/utils/constants.js). Mirror the H7172 setup
        // until per-model behavior is verified.
        Quirk::ice_maker("H717D").with_iot_api_support(false),
        Quirk::ice_maker("H8120").with_iot_api_support(false),
        // Dual smart plug. The platform API only exposes a single combined
        // powerSwitch, but the IoT status packet reports each outlet as one
        // bit of the onOff value, so we can report per-outlet state.
        // <https://github.com/wez/govee2mqtt/issues/65>
        Quirk::device("H5082", DeviceType::Socket, POWER_SOCKET).with_socket_outlets(2),
        // Triple smart plug; same per-outlet bitfield situation as H5082.
        // Grouped under homebridge-govee's `switchTriple` bucket
        // (lib/utils/constants.js).
        Quirk::device("H5160", DeviceType::Socket, POWER_SOCKET).with_socket_outlets(3),
        // Single Wi-Fi smart plugs/switches. The platform API returns no
        // metadata for these, so without a quirk they map to nothing; with one,
        // they get a synthesized powerSwitch driven over IoT. iot_api_support is
        // required for control to route over IoT (the only available transport).
        Quirk::device("H5080", DeviceType::Socket, POWER_SOCKET).with_iot_api_support(true),
        Quirk::device("H5083", DeviceType::Socket, POWER_SOCKET).with_iot_api_support(true),
        // Additional single-outlet smart plugs grouped with H5080/H5083 in
        // homebridge-govee's `switchSingle` bucket (lib/utils/constants.js).
        Quirk::device("H5001", DeviceType::Socket, POWER_SOCKET).with_iot_api_support(true),
        Quirk::device("H5081", DeviceType::Socket, POWER_SOCKET).with_iot_api_support(true),
        Quirk::device("H5086", DeviceType::Socket, POWER_SOCKET).with_iot_api_support(true),
        Quirk::device("H7014", DeviceType::Socket, POWER_SOCKET).with_iot_api_support(true),
        // H6093 "Stars" aurora/laser projector. The platform API lists it as a
        // light but reports no brightness/color capability, so without this it
        // surfaces as a bare power switch. The device takes cmd:"turn" and
        // cmd:"brightness" over IoT (status reports onOff + brightness), so mark
        // it a brightness light driven over IoT; its aurora/laser/settings
        // controls are synthesized separately (synthesize_h6093_capabilities). No
        // master RGB/color-temp: the app only exposes per-layer colors, not a
        // master color picker.
        Quirk::device("H6093", DeviceType::Light, PROJECTOR)
            .with_brightness()
            .with_iot_api_support(true),
        Quirk::thermometer("H5051")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit)
            .with_platform_humidity_sensor_units(HumidityUnits::RelativePercent),
        Quirk::thermometer("H5100")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit)
            .with_platform_humidity_sensor_units(HumidityUnits::RelativePercent),
        Quirk::thermometer("H5103")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit)
            .with_platform_humidity_sensor_units(HumidityUnits::RelativePercent),
        Quirk::thermometer("H5179")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit)
            .with_platform_humidity_sensor_units(HumidityUnits::RelativePercent),
        Quirk::device("H7170", DeviceType::Kettle, "mdi:kettle")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit),
        Quirk::device("H7171", DeviceType::Kettle, "mdi:kettle")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit)
            .with_show_as_preset_modes(&["M1", "M2", "M3", "M4"]),
        Quirk::device("H7173", DeviceType::Kettle, "mdi:kettle")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit)
            .with_show_as_preset_modes(&["Tea", "Coffee", "DIY"]),
        // Additional kettles grouped with the H717x series in homebridge-govee's
        // `kettle` bucket (lib/utils/constants.js). Per-SKU preset mode lists
        // unverified; left off until we know them.
        Quirk::device("H7175", DeviceType::Kettle, "mdi:kettle")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit),
        Quirk::device("H717A", DeviceType::Kettle, "mdi:kettle")
            .with_platform_temperature_sensor_units(TemperatureUnits::Fahrenheit),
        // Lights from the list of LAN API enabled devices
        // at <https://app-h5.govee.com/user-manual/wlan-guide>
        Quirk::lan_api_capable_light("H6072", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H619B", STRIP),
        Quirk::lan_api_capable_light("H619C", STRIP),
        Quirk::lan_api_capable_light("H619Z", STRIP),
        Quirk::lan_api_capable_light("H7060", FLOOD),
        // Additional LAN-API capable lights from the official Govee LAN guide
        // at <https://app-h5.govee.com/user-manual/wlan-guide>. Grouped by
        // class. Icons picked from the closest existing match; refine per-SKU
        // if a user reports a wrong icon.
        // Floor lamps
        Quirk::lan_api_capable_light("H1630", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H16B0", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H16C0", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H607C", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H60B0", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H60B1", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H60B2", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H60B3", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H8076", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H807C", FLOOR_LAMP),
        // Table lamps (H6022 already covered above with a tighter color-temp range)
        Quirk::lan_api_capable_light("H6020", DESK),
        Quirk::lan_api_capable_light("H8022", DESK),
        // Gaming / pixel displays and gaming light bars
        Quirk::lan_api_capable_light("H6048", TV_BACK),
        Quirk::lan_api_capable_light("H8048", TV_BACK),
        Quirk::lan_api_capable_light("H6630", STRIP),
        Quirk::lan_api_capable_light("H6631", STRIP),
        Quirk::lan_api_capable_light("H8630", STRIP),
        // Ceiling lights (H60A1 covered above with custom temp range)
        Quirk::lan_api_capable_light("H1232", CEILING),
        Quirk::lan_api_capable_light("H1252", CEILING),
        Quirk::lan_api_capable_light("H1270", CEILING),
        Quirk::lan_api_capable_light("H60A4", CEILING),
        Quirk::lan_api_capable_light("H60A6", CEILING),
        Quirk::lan_api_capable_light("H80A1", CEILING),
        Quirk::lan_api_capable_light("H80A4", CEILING),
        Quirk::lan_api_capable_light("H12D0", CEILING),
        // Pendant light (hangs from ceiling, closest icon)
        Quirk::lan_api_capable_light("H60C1", CEILING),
        // Wall sconces and panel lights
        Quirk::lan_api_capable_light("H6038", WALL_SCONCE),
        Quirk::lan_api_capable_light("H6039", WALL_SCONCE),
        Quirk::lan_api_capable_light("H6088", WALL_SCONCE),
        Quirk::lan_api_capable_light("H6069", HEX),
        Quirk::lan_api_capable_light("H606A", HEX),
        Quirk::lan_api_capable_light("H8069", HEX),
        // Smart bulbs (RGBWW)
        Quirk::lan_api_capable_light("H1401", BULB),
        Quirk::lan_api_capable_light("H14A1", BULB),
        Quirk::lan_api_capable_light("H14C2", BULB),
        Quirk::lan_api_capable_light("H6004", BULB),
        Quirk::lan_api_capable_light("H6006", BULB),
        Quirk::lan_api_capable_light("H6008", BULB),
        Quirk::lan_api_capable_light("H6009", BULB),
        Quirk::lan_api_capable_light("H600A", BULB),
        Quirk::lan_api_capable_light("H6010", BULB),
        Quirk::lan_api_capable_light("H8015", BULB),
        // String / Christmas lights
        Quirk::lan_api_capable_light("H6800", STRING),
        Quirk::lan_api_capable_light("H6870", STRING),
        Quirk::lan_api_capable_light("H6871", STRING),
        Quirk::lan_api_capable_light("H608A", STRING),
        // Outdoor spotlights and pathway lights
        Quirk::lan_api_capable_light("H3200", SPOTLIGHT),
        Quirk::lan_api_capable_light("H3500", OUTDOOR_LAMP),
        Quirk::lan_api_capable_light("H3501", OUTDOOR_LAMP),
        // Recessed downlights (no dedicated downlight icon; spotlight is closest)
        Quirk::lan_api_capable_light("H601A", SPOTLIGHT),
        Quirk::lan_api_capable_light("H601B", SPOTLIGHT),
        Quirk::lan_api_capable_light("H601C", SPOTLIGHT),
        Quirk::lan_api_capable_light("H801A", SPOTLIGHT),
        Quirk::lan_api_capable_light("H801D", SPOTLIGHT),
        // Sibling Star Light Projectors to H6093 (Ocean Wave, Nebula, Galaxy
        // 2 Pro). The H6093 was registered with brightness-only + IoT because
        // its platform API reports no master color picker (only per-layer
        // aurora/laser controls). Mirror that shape for the siblings and add
        // LAN since Govee's list includes them; if their capability shape is
        // richer than H6093's the platform API will surface the extras
        // alongside this quirk.
        Quirk::device("H6094", DeviceType::Light, PROJECTOR)
            .with_brightness()
            .with_iot_api_support(true)
            .with_lan_api(),
        Quirk::device("H6095", DeviceType::Light, PROJECTOR)
            .with_brightness()
            .with_iot_api_support(true)
            .with_lan_api(),
        Quirk::device("H609D", DeviceType::Light, PROJECTOR)
            .with_brightness()
            .with_iot_api_support(true)
            .with_lan_api(),
        // Ceiling fans with integrated lights. We do not have a dedicated fan
        // device class yet, so only the LIGHT side is reachable; the fan
        // blades will need fan-class wiring to expose. Icon hints at the
        // combined form factor.
        Quirk::lan_api_capable_light("H1310", CEILING_FAN),
        Quirk::lan_api_capable_light("H1370", CEILING_FAN),
        // Outdoor pathway / wall / cylinder lights
        Quirk::lan_api_capable_light("H3510", OUTDOOR_LAMP),
        Quirk::lan_api_capable_light("H3511", OUTDOOR_LAMP),
        Quirk::lan_api_capable_light("H3751", WALL_SCONCE),
        // More strip variants from the definitive list (Skyline Kit, Strip
        // Light 2 Pro, Neon Rope 2, additional cover/M1 variants)
        Quirk::lan_api_capable_light("H61B6", STRIP),
        Quirk::lan_api_capable_light("H61B8", STRIP),
        Quirk::lan_api_capable_light("H61B9", STRIP),
        Quirk::lan_api_capable_light("H61C3", STRIP),
        Quirk::lan_api_capable_light("H61D3", STRIP),
        Quirk::lan_api_capable_light("H61D5", STRIP),
        Quirk::lan_api_capable_light("H61F2", STRIP),
        Quirk::lan_api_capable_light("H61F5", STRIP),
        // Christmas / decorative strings (H6811 is 480LED Net Lights —
        // sibling of H6810; H70Bx/H70Cx/H70Dx are curtain / christmas /
        // icicle variants)
        Quirk::lan_api_capable_light("H6811", STRING),
        Quirk::lan_api_capable_light("H70B6", STRING),
        Quirk::lan_api_capable_light("H70B8", STRIP),
        Quirk::lan_api_capable_light("H70C4", STRING),
        Quirk::lan_api_capable_light("H70C5", STRING),
        Quirk::lan_api_capable_light("H70C7", STRING),
        Quirk::lan_api_capable_light("H70D1", STRING),
        Quirk::lan_api_capable_light("H70D2", STRING),
        // Outdoor string light variants (S14 bulb, dots, permanent outdoor
        // 2 / 2 Pro / Prism, garden, tree, chromatic)
        Quirk::lan_api_capable_light("H7025", STRING),
        Quirk::lan_api_capable_light("H7026", STRING),
        Quirk::lan_api_capable_light("H702A", STRING),
        Quirk::lan_api_capable_light("H702B", STRING),
        Quirk::lan_api_capable_light("H702C", STRING),
        Quirk::lan_api_capable_light("H7037", STRING),
        Quirk::lan_api_capable_light("H7038", STRING),
        Quirk::lan_api_capable_light("H7039", STRING),
        Quirk::lan_api_capable_light("H703B", STRING),
        Quirk::lan_api_capable_light("H7046", STRING),
        Quirk::lan_api_capable_light("H705D", STRING),
        Quirk::lan_api_capable_light("H705E", STRING),
        Quirk::lan_api_capable_light("H705F", STRING),
        Quirk::lan_api_capable_light("H706A", STRING),
        Quirk::lan_api_capable_light("H706B", STRING),
        Quirk::lan_api_capable_light("H706C", STRING),
        Quirk::lan_api_capable_light("H707A", STRING),
        Quirk::lan_api_capable_light("H707B", STRING),
        Quirk::lan_api_capable_light("H707C", STRING),
        Quirk::lan_api_capable_light("H7086", STRING),
        Quirk::lan_api_capable_light("H7087", STRING),
        // Outdoor flood / pathway / lamp post / wall / spotlights
        Quirk::lan_api_capable_light("H7056", OUTDOOR_LAMP),
        Quirk::lan_api_capable_light("H7057", FLOOD),
        Quirk::lan_api_capable_light("H7058", FLOOD),
        Quirk::lan_api_capable_light("H7071", SPOTLIGHT),
        Quirk::lan_api_capable_light("H7072", OUTDOOR_LAMP),
        Quirk::lan_api_capable_light("H7073", SPOTLIGHT),
        Quirk::lan_api_capable_light("H7076", WALL_SCONCE),
        Quirk::lan_api_capable_light("H7093", SPOTLIGHT),
        Quirk::lan_api_capable_light("H7094", SPOTLIGHT),
        // Second pass through the LAN guide page picking up SKUs the first
        // pass missed (page content past the initial visible region). Same
        // source: <https://app-h5.govee.com/user-manual/wlan-guide>.
        // Strip/general lights (default STRIP icon; refine if a user reports
        // a wrong form-factor). H6052 and H615B also have BLE-transport
        // quirks documented elsewhere; those layer independently on top of
        // LAN registration.
        Quirk::lan_api_capable_light("H6042", STRIP),
        Quirk::lan_api_capable_light("H6043", STRIP),
        Quirk::lan_api_capable_light("H6052", STRIP),
        Quirk::lan_api_capable_light("H6063", WALL_SCONCE),
        Quirk::lan_api_capable_light("H6110", STRIP),
        Quirk::lan_api_capable_light("H6143", STRIP),
        Quirk::lan_api_capable_light("H6144", STRIP),
        Quirk::lan_api_capable_light("H615A", STRIP),
        Quirk::lan_api_capable_light("H615B", STRIP),
        Quirk::lan_api_capable_light("H615C", STRIP),
        Quirk::lan_api_capable_light("H615D", STRIP),
        Quirk::lan_api_capable_light("H616C", STRIP),
        Quirk::lan_api_capable_light("H616D", STRIP),
        Quirk::lan_api_capable_light("H616E", STRIP),
        Quirk::lan_api_capable_light("H6167", STRIP),
        Quirk::lan_api_capable_light("H6182", STRIP),
        Quirk::lan_api_capable_light("H618G", STRIP),
        Quirk::lan_api_capable_light("H61A9", STRIP),
        Quirk::lan_api_capable_light("H61B1", STRIP),
        Quirk::lan_api_capable_light("H61B3", STRIP),
        Quirk::lan_api_capable_light("H61BA", STRIP),
        Quirk::lan_api_capable_light("H61BC", STRIP),
        Quirk::lan_api_capable_light("H61BE", STRIP),
        Quirk::lan_api_capable_light("H61C2", STRIP),
        Quirk::lan_api_capable_light("H61C5", STRIP),
        Quirk::lan_api_capable_light("H61D6", STRIP),
        Quirk::lan_api_capable_light("H61E0", STRIP),
        Quirk::lan_api_capable_light("H61F6", STRIP),
        // H6671/H6672 are "RGBWIC TV Backlight 2" per Govee's definitive
        // LAN list, not panel lights — TV backlight icon is the right fit.
        Quirk::lan_api_capable_light("H6671", TV_BACK),
        Quirk::lan_api_capable_light("H6672", TV_BACK),
        // Pendant/ceiling adjacent (H60A0 sits next to H60A1/H60A4/H60A6)
        Quirk::lan_api_capable_light("H60A0", CEILING),
        // Per Govee's definitive LAN list: H30D0/H30D1 are Outdoor Filament
        // String Lights, H3A5x are Permanent Outdoor Lights 2 Pro (string),
        // H6350/H6351 are Sphere Net Lights (string). All take STRING.
        Quirk::lan_api_capable_light("H30D0", STRING),
        Quirk::lan_api_capable_light("H30D1", STRING),
        Quirk::lan_api_capable_light("H3A51", STRING),
        Quirk::lan_api_capable_light("H3A52", STRING),
        Quirk::lan_api_capable_light("H3A53", STRING),
        Quirk::lan_api_capable_light("H6350", STRING),
        Quirk::lan_api_capable_light("H6351", STRING),
        // String / Christmas / curtain lights (H68xx, H7xxx, H80xx in those
        // ranges; existing H68xx and H7000-series entries are STRING).
        Quirk::lan_api_capable_light("H6810", STRING),
        Quirk::lan_api_capable_light("H6840", STRING),
        Quirk::lan_api_capable_light("H6841", STRING),
        Quirk::lan_api_capable_light("H6842", STRING),
        Quirk::lan_api_capable_light("H6843", STRING),
        Quirk::lan_api_capable_light("H6850", STRING),
        Quirk::lan_api_capable_light("H6860", STRING),
        Quirk::lan_api_capable_light("H6861", STRING),
        Quirk::lan_api_capable_light("H7027", STRING),
        Quirk::lan_api_capable_light("H7030", STRING),
        Quirk::lan_api_capable_light("H7033", STRING),
        Quirk::lan_api_capable_light("H703A", STRING),
        Quirk::lan_api_capable_light("H7045", STRING),
        Quirk::lan_api_capable_light("H7063", STRING),
        Quirk::lan_api_capable_light("H7066", STRING),
        Quirk::lan_api_capable_light("H7070", STRING),
        Quirk::lan_api_capable_light("H70B1", STRING),
        Quirk::lan_api_capable_light("H70BC", STRING),
        Quirk::lan_api_capable_light("H70C1", STRING),
        Quirk::lan_api_capable_light("H70C2", STRING),
        Quirk::lan_api_capable_light("H70C9", STRING),
        Quirk::lan_api_capable_light("H70D3", STRING),
        Quirk::lan_api_capable_light("H801B", STRING),
        Quirk::lan_api_capable_light("H802A", STRING),
        Quirk::lan_api_capable_light("H8025", STRING),
        Quirk::lan_api_capable_light("H8026", STRING),
        Quirk::lan_api_capable_light("H8057", STRING),
        Quirk::lan_api_capable_light("H805A", STRING),
        Quirk::lan_api_capable_light("H805B", STRING),
        Quirk::lan_api_capable_light("H805C", STRING),
        Quirk::lan_api_capable_light("H8066", STRING),
        Quirk::lan_api_capable_light("H806C", STRING),
        Quirk::lan_api_capable_light("H80C4", STRING),
        Quirk::lan_api_capable_light("H80C5", STRING),
        Quirk::lan_api_capable_light("H80D1", STRING),
        Quirk::lan_api_capable_light("H8811", STRING),
        Quirk::lan_api_capable_light("H8840", STRING),
        Quirk::lan_api_capable_light("H8841", STRING),
        // H1Axx / H1ABx / H1B6A — Govee's definitive LAN list shows these
        // are all strip variants (LED Strip Light 2, COB Strip Light 2,
        // Strip Light with Tower Kit).
        Quirk::lan_api_capable_light("H1A43", STRIP),
        Quirk::lan_api_capable_light("H1A44", STRIP),
        Quirk::lan_api_capable_light("H1A45", STRIP),
        Quirk::lan_api_capable_light("H1AB1", STRIP),
        Quirk::lan_api_capable_light("H1AB2", STRIP),
        Quirk::lan_api_capable_light("H1AB3", STRIP),
        Quirk::lan_api_capable_light("H1B6A", STRIP),
        Quirk::lan_api_capable_light("H6046", TV_BACK),
        Quirk::lan_api_capable_light("H6047", TV_BACK),
        Quirk::lan_api_capable_light("H6051", DESK),
        Quirk::lan_api_capable_light("H6056", STRIP_ALT),
        Quirk::lan_api_capable_light("H6059", NIGHTLIGHT),
        Quirk::lan_api_capable_light("H6061", HEX),
        Quirk::lan_api_capable_light("H6062", STRIP),
        Quirk::lan_api_capable_light("H6065", STRIP),
        Quirk::lan_api_capable_light("H6066", HEX),
        Quirk::lan_api_capable_light("H6067", TRIANGLE),
        Quirk::lan_api_capable_light("H6073", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H6076", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H6078", FLOOR_LAMP),
        Quirk::lan_api_capable_light("H6087", WALL_SCONCE),
        Quirk::lan_api_capable_light("H610A", STRIP),
        Quirk::lan_api_capable_light("H610B", STRIP),
        Quirk::lan_api_capable_light("H6117", STRIP),
        Quirk::lan_api_capable_light("H6159", STRIP),
        Quirk::lan_api_capable_light("H615E", STRIP),
        Quirk::lan_api_capable_light("H6163", STRIP),
        Quirk::lan_api_capable_light("H6168", TV_BACK),
        Quirk::lan_api_capable_light("H6172", STRIP),
        Quirk::lan_api_capable_light("H6173", STRIP),
        Quirk::lan_api_capable_light("H618A", STRIP),
        Quirk::lan_api_capable_light("H618C", STRIP),
        Quirk::lan_api_capable_light("H618E", STRIP),
        Quirk::lan_api_capable_light("H618F", STRIP),
        Quirk::lan_api_capable_light("H619A", STRIP),
        Quirk::lan_api_capable_light("H619D", STRIP),
        Quirk::lan_api_capable_light("H619E", STRIP),
        Quirk::lan_api_capable_light("H61A0", STRIP),
        Quirk::lan_api_capable_light("H61A1", STRIP),
        Quirk::lan_api_capable_light("H61A2", STRIP),
        Quirk::lan_api_capable_light("H61A3", STRIP),
        Quirk::lan_api_capable_light("H61A5", STRIP),
        Quirk::lan_api_capable_light("H61A8", STRIP),
        Quirk::lan_api_capable_light("H61B2", TV_BACK),
        Quirk::lan_api_capable_light("H61E1", STRIP),
        Quirk::lan_api_capable_light("H7012", STRING),
        Quirk::lan_api_capable_light("H7013", STRING),
        Quirk::lan_api_capable_light("H7021", STRING),
        Quirk::lan_api_capable_light("H7028", STRING),
        Quirk::lan_api_capable_light("H7041", STRING),
        Quirk::lan_api_capable_light("H7042", STRING),
        Quirk::lan_api_capable_light("H7050", BULB),
        Quirk::lan_api_capable_light("H7051", BULB),
        Quirk::lan_api_capable_light("H7052", STRING),
        Quirk::lan_api_capable_light("H7055", BULB),
        Quirk::lan_api_capable_light("H705A", OUTDOOR_LAMP),
        Quirk::lan_api_capable_light("H705B", OUTDOOR_LAMP),
        Quirk::lan_api_capable_light("H7061", FLOOD),
        Quirk::lan_api_capable_light("H7062", FLOOD),
        Quirk::lan_api_capable_light("H7065", SPOTLIGHT),
    ] {
        map.insert(quirk.sku.to_string(), quirk);
    }

    map
}

pub fn resolve_quirk(sku: &str) -> Option<&'static Quirk> {
    QUIRKS.get(sku)
}
