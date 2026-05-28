use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::humidifier::DEVICE_CLASS_HUMIDITY;
use crate::hass_mqtt::instance::{Component, EntityInstance, component};
use crate::hass_mqtt::topic::Topics;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::{
    HassClient, camel_case_to_space_separated, topic_safe_id, topic_safe_string,
};
use crate::service::quirks::HumidityUnits;
use crate::service::state::StateHandle;
use async_trait::async_trait;
use govee_api::platform_api::DeviceCapability;
use govee_api::temperature::{DEVICE_CLASS_TEMPERATURE, TemperatureUnits, TemperatureValue};
use serde::Serialize;
use serde_json::json;

#[derive(Serialize, Clone, Debug)]
pub struct SensorConfig {
    #[serde(flatten)]
    pub base: EntityConfig,

    pub state_topic: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_class: Option<StateClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_of_measurement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_attributes_topic: Option<String>,
}

#[allow(unused)]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateClass {
    #[serde(rename = "measurement")]
    Measurement,
    #[serde(rename = "total")]
    Total,
    #[serde(rename = "total_increasing")]
    TotalIncreasing,
}

impl SensorConfig {
    fn component(&self) -> Component {
        component("sensor", &self.base, self)
    }

    pub async fn notify_state(&self, client: &HassClient, value: &str) -> anyhow::Result<()> {
        client.publish(&self.state_topic, value).await
    }
}

/// Home Assistant metadata for a Govee property/sensor capability, keyed by its
/// instance name. device_class drives the icon and unit handling, state_class
/// tells HA to keep long-term statistics, and name overrides the default
/// camel-case-derived label where a nicer one exists.
struct SensorMeta {
    device_class: Option<&'static str>,
    state_class: Option<StateClass>,
    unit: Option<&'static str>,
    name: String,
}

impl SensorMeta {
    fn for_instance(instance: &str) -> Self {
        let (device_class, state_class, unit, name) = match instance {
            // sensorTemperature's unit follows the configured scale and is set
            // by the caller; device_class/state_class are still fixed here.
            "sensorTemperature" => (
                Some(DEVICE_CLASS_TEMPERATURE),
                Some(StateClass::Measurement),
                None,
                "Temperature",
            ),
            "humidity" | "sensorHumidity" => (
                Some(DEVICE_CLASS_HUMIDITY),
                Some(StateClass::Measurement),
                Some("%"),
                "Humidity",
            ),
            // The H5140 CO2 monitor reports carbonDioxideConcentration in ppm.
            "carbonDioxideConcentration" => (
                Some("carbon_dioxide"),
                Some(StateClass::Measurement),
                Some("ppm"),
                "CO2",
            ),
            // airQuality and filterLifeTime are reported by Govee without a
            // declared unit, and the device_class differs by model (pm2.5
            // concentration vs an index, percent-remaining vs hours), so we
            // only mark them as measurements and let HA infer the rest.
            "airQuality" => (None, Some(StateClass::Measurement), None, "Air Quality"),
            "filterLifeTime" => (None, Some(StateClass::Measurement), None, "Filter Life"),
            "online" => (None, None, None, "Connected to Govee Cloud"),
            _ => {
                return Self {
                    device_class: None,
                    state_class: None,
                    unit: None,
                    name: camel_case_to_space_separated(instance),
                };
            }
        };

        Self {
            device_class,
            state_class,
            unit,
            name: name.to_string(),
        }
    }
}

#[derive(Clone)]
pub struct GlobalFixedDiagnostic {
    sensor: SensorConfig,
    value: String,
}

#[async_trait]
impl EntityInstance for GlobalFixedDiagnostic {
    fn component(&self) -> Component {
        self.sensor.component()
    }

    async fn notify_state(
        &self,
        _device: Option<&ServiceDevice>,
        client: &HassClient,
    ) -> anyhow::Result<()> {
        self.sensor.notify_state(client, &self.value).await
    }
}

impl GlobalFixedDiagnostic {
    pub fn new<NAME: Into<String>, VALUE: Into<String>>(
        topics: &Topics,
        name: NAME,
        value: VALUE,
    ) -> Self {
        let name = name.into();
        let unique_id = format!("global-{}", topic_safe_string(&name));
        let (availability, availability_mode) = EntityConfig::global_availability(topics);

        Self {
            sensor: SensorConfig {
                base: EntityConfig {
                    availability,
                    availability_mode,
                    name: Some(name),
                    entity_category: Some("diagnostic".to_string()),
                    origin: Origin::default(),
                    device: Device::this_service(topics),
                    unique_id: unique_id.clone(),
                    device_class: None,
                    icon: None,
                },
                state_topic: topics.sensor_state(&unique_id),
                state_class: None,
                unit_of_measurement: None,
                json_attributes_topic: None,
            },
            value: value.into(),
        }
    }
}

#[derive(Clone)]
pub struct CapabilitySensor {
    sensor: SensorConfig,
    device_id: String,
    state: StateHandle,
    instance_name: String,
}

impl CapabilitySensor {
    pub async fn new(
        topics: &Topics,
        device: &ServiceDevice,
        state: &StateHandle,
        instance: &DeviceCapability,
    ) -> anyhow::Result<Self> {
        let unique_id = format!(
            "sensor-{id}-{inst}",
            id = topic_safe_id(device),
            inst = topic_safe_string(&instance.instance)
        );

        let meta = SensorMeta::for_instance(&instance.instance);

        // Temperature unit follows the user's configured scale, so it can't be
        // a static table entry like the others. For everything else, prefer
        // the SensorMeta table (static, curated), then fall back to the unit
        // declared on the capability itself (eg synthesized H5082 countdown
        // remaining-seconds carries `unit: "s"` in its parameters).
        let unit_of_measurement = if instance.instance == "sensorTemperature" {
            Some(
                state
                    .get_temperature_scale()
                    .await
                    .unit_of_measurement()
                    .to_string(),
            )
        } else {
            meta.unit
                .map(str::to_string)
                .or_else(|| match &instance.parameters {
                    Some(govee_api::model::DeviceParameters::Integer { unit, .. }) => unit.clone(),
                    _ => None,
                })
        };
        let device_class = meta.device_class;
        let state_class = meta.state_class;
        let name = meta.name;

        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);

        Ok(Self {
            sensor: SensorConfig {
                base: EntityConfig {
                    availability,
                    availability_mode,
                    name: Some(name),
                    entity_category: Some("diagnostic".to_string()),
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id: unique_id.clone(),
                    device_class,
                    icon: None,
                },
                state_topic: topics.sensor_state(&unique_id),
                state_class,
                unit_of_measurement,
                json_attributes_topic: None,
            },
            device_id: device.id.to_string(),
            state: state.clone(),
            instance_name: instance.instance.to_string(),
        })
    }
}

#[async_trait]
impl EntityInstance for CapabilitySensor {
    fn component(&self) -> Component {
        self.sensor.component()
    }

    fn device_id(&self) -> Option<&str> {
        Some(&self.device_id)
    }

    async fn notify_state(
        &self,
        device: Option<&ServiceDevice>,
        client: &HassClient,
    ) -> anyhow::Result<()> {
        let Some(device) = device else { return Ok(()) };

        let quirk = device.resolve_quirk();

        if let Some(cap) = device.get_state_capability_by_instance(&self.instance_name) {
            let value = match self.instance_name.as_str() {
                "sensorTemperature" => {
                    let units = quirk
                        .and_then(|q| q.platform_temperature_sensor_units)
                        .unwrap_or(TemperatureUnits::Fahrenheit);

                    match cap
                        .state
                        .pointer("/value")
                        .and_then(|v| v.as_f64())
                        .map(|v| TemperatureValue::new(v, units))
                    {
                        Some(v) => {
                            let value = v
                                .as_unit(self.state.get_temperature_scale().await.into())
                                .value();
                            format!("{value:.2}")
                        }
                        None => "".to_string(),
                    }
                }
                "sensorHumidity" => {
                    let units = quirk
                        .and_then(|q| q.platform_humidity_sensor_units)
                        .unwrap_or(HumidityUnits::RelativePercent);
                    match cap
                        .state
                        .pointer("/value")
                        .and_then(|v| v.as_f64())
                        .map(|v| units.from_reading_to_relative_percent(v))
                    {
                        Some(v) => format!("{v:.2}"),
                        None => "".to_string(),
                    }
                }
                _ => cap
                    .state
                    .pointer("/value")
                    .map(|v| match v {
                        // Pull the numeric or string value out of the
                        // synthesized state envelope so HA displays just the
                        // value (eg `53388`) rather than `{"value":53388}`.
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_else(|| cap.state.to_string()),
            };

            return self.sensor.notify_state(client, &value).await;
        }
        log::trace!(
            "CapabilitySensor::notify_state: didn't find state for {device} {instance}",
            instance = self.instance_name
        );
        Ok(())
    }
}

/// A read-only sensor for a platform-API Event capability (eg: a leak or filter
/// alarm). Govee gives no way to set these, so there's no command path. The
/// latest reported value is the sensor state; the alarm type and the raw event
/// payload are exposed as attributes for full granularity.
pub struct CapabilityEventSensor {
    sensor: SensorConfig,
    device_id: String,
    instance_name: String,
}

impl CapabilityEventSensor {
    pub fn new(topics: &Topics, device: &ServiceDevice, instance: &DeviceCapability) -> Self {
        let unique_id = format!(
            "sensor-{id}-event-{inst}",
            id = topic_safe_id(device),
            inst = topic_safe_string(&instance.instance)
        );

        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);

        Self {
            sensor: SensorConfig {
                base: EntityConfig {
                    availability,
                    availability_mode,
                    name: Some(crate::service::hass::camel_case_to_space_separated(
                        &instance.instance,
                    )),
                    entity_category: Some("diagnostic".to_string()),
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id: unique_id.clone(),
                    device_class: None,
                    icon: Some("mdi:bell-alert".to_string()),
                },
                state_topic: topics.sensor_state(&unique_id),
                state_class: None,
                unit_of_measurement: None,
                json_attributes_topic: Some(topics.sensor_attributes(&unique_id)),
            },
            device_id: device.id.to_string(),
            instance_name: instance.instance.to_string(),
        }
    }
}

#[async_trait]
impl EntityInstance for CapabilityEventSensor {
    fn component(&self) -> Component {
        self.sensor.component()
    }

    fn device_id(&self) -> Option<&str> {
        Some(&self.device_id)
    }

    async fn notify_state(
        &self,
        device: Option<&ServiceDevice>,
        client: &HassClient,
    ) -> anyhow::Result<()> {
        let Some(device) = device else { return Ok(()) };

        if let Some(cap) = device.get_state_capability_by_instance(&self.instance_name) {
            // Prefer a scalar /value for the sensor state, falling back to the
            // whole state as a string when there isn't one.
            let summary = match cap.state.pointer("/value") {
                Some(v) if v.is_string() => v.as_str().unwrap_or_default().to_string(),
                Some(v) if !v.is_null() => v.to_string(),
                _ => cap.state.to_string(),
            };
            self.sensor.notify_state(client, &summary).await?;
            if let Some(topic) = &self.sensor.json_attributes_topic {
                client
                    .publish_obj(topic, json!({ "raw_state": cap.state }))
                    .await?;
            }
            return Ok(());
        }

        // No event reported yet, also expose the static metadata (alarm type)
        // from the capability definition so the entity carries some context.
        if let Some(cap) = device.get_capability_by_instance(&self.instance_name)
            && let Some(topic) = &self.sensor.json_attributes_topic
        {
            client
                .publish_obj(
                    topic,
                    json!({
                        "alarm_type": cap.alarm_type,
                        "event_state": cap.event_state,
                    }),
                )
                .await?;
        }
        Ok(())
    }
}

pub struct DeviceStatusDiagnostic {
    sensor: SensorConfig,
    device_id: String,
}

impl DeviceStatusDiagnostic {
    pub fn new(topics: &Topics, device: &ServiceDevice) -> Self {
        let unique_id = topics.status_sensor_id(device);
        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);

        Self {
            sensor: SensorConfig {
                base: EntityConfig {
                    availability,
                    availability_mode,
                    name: Some("Status".to_string()),
                    entity_category: Some("diagnostic".to_string()),
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id: unique_id.clone(),
                    device_class: None,
                    icon: None,
                },
                state_topic: topics.sensor_state(&unique_id),
                state_class: None,
                json_attributes_topic: Some(topics.sensor_attributes(&unique_id)),
                unit_of_measurement: None,
            },
            device_id: device.id.to_string(),
        }
    }
}

#[async_trait]
impl EntityInstance for DeviceStatusDiagnostic {
    fn component(&self) -> Component {
        self.sensor.component()
    }

    fn device_id(&self) -> Option<&str> {
        Some(&self.device_id)
    }

    async fn notify_state(
        &self,
        device: Option<&ServiceDevice>,
        client: &HassClient,
    ) -> anyhow::Result<()> {
        let Some(device) = device else { return Ok(()) };

        let iot_state = device.compute_iot_device_state();
        let lan_state = device.compute_lan_device_state();
        let http_state = device.compute_http_device_state();
        let platform_metadata = &device.http_device_info;
        let platform_state = &device.http_device_state;
        let device_state = device.device_state();

        let summary = device.availability_status().as_status_text().to_string();

        let attributes = json!({
            "iot": iot_state,
            "lan": lan_state,
            "http": http_state,
            "platform_metadata": platform_metadata,
            "platform_state": platform_state,
            "overall": device_state,
            "room_name": device.undoc_device_info.as_ref().and_then(|i| i.room_name.clone()),
            "nightlight": device.nightlight_state,
        });

        self.sensor.notify_state(client, &summary).await?;
        if let Some(topic) = &self.sensor.json_attributes_topic {
            client.publish_obj(topic, attributes).await?;
        }
        Ok(())
    }
}
