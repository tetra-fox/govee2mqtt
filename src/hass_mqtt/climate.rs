use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::instance::EntityInstance;
use crate::hass_mqtt::number::NumberConfig;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::{availability_topic, topic_safe_id, topic_safe_string, HassClient};
use crate::service::state::StateHandle;
use govee_api::platform_api::{parse_temperature_constraints, DeviceCapability};
use govee_api::temperature::{
    TemperatureScale, TemperatureUnits, TemperatureValue, DEVICE_CLASS_TEMPERATURE,
};
use mosquitto_rs::router::{Params, Payload, State};
use serde::Deserialize;
use std::str::FromStr;

// TODO: register an actual climate entity.
// I don't have one of these devices, so it is currently guesswork!

pub struct TargetTemperatureEntity {
    number: NumberConfig,
    device_id: String,
    state: StateHandle,
    instance_name: String,
}

impl TargetTemperatureEntity {
    pub async fn new(
        base_topic: &str,
        device: &ServiceDevice,
        state: &StateHandle,
        instance: &DeviceCapability,
    ) -> anyhow::Result<Self> {
        let units = state.get_temperature_scale().await;

        let constraints = parse_temperature_constraints(instance)?.as_unit(units.into());
        let unique_id = format!(
            "{id}-{inst}",
            id = topic_safe_id(device),
            inst = topic_safe_string(&instance.instance)
        );

        let name = "Target Temperature".to_string();
        let command_topic = format!(
            "{base_topic}/{id}/set-temperature/{inst}/{units}",
            id = topic_safe_id(device),
            inst = topic_safe_string(&instance.instance)
        );
        let state_topic = format!(
            "{base_topic}/{id}/advise-set-temperature",
            id = topic_safe_id(device),
        );

        Ok(Self {
            number: NumberConfig {
                base: EntityConfig {
                    availability_topic: availability_topic(base_topic),
                    name: Some(name),
                    entity_category: None,
                    origin: Origin::default(),
                    device: Device::for_device(base_topic, device),
                    unique_id: unique_id.clone(),
                    device_class: Some(DEVICE_CLASS_TEMPERATURE),
                    icon: Some("mdi:thermometer".to_string()),
                },
                state_topic: Some(state_topic),
                command_topic,
                min: Some(constraints.min.value().floor() as f32),
                max: Some(constraints.max.value().ceil() as f32),
                step: 1.0,
                unit_of_measurement: Some(units.unit_of_measurement()),
            },
            device_id: device.id.to_string(),
            state: state.clone(),
            instance_name: instance.instance.to_string(),
        })
    }
}

#[async_trait::async_trait]
impl EntityInstance for TargetTemperatureEntity {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        self.number.publish(state, client).await
    }

    async fn notify_state(&self, client: &HassClient) -> anyhow::Result<()> {
        let device = self
            .state
            .device_by_id(&self.device_id)
            .await
            .expect("device to exist");

        let quirk = device.resolve_quirk();

        log::debug!("notify_state for {device} {}", self.instance_name);

        if let Some(cap) = device.get_state_capability_by_instance(&self.instance_name) {
            log::debug!("have: {cap:?}");

            let units = cap
                .state
                .pointer("/value/unit")
                .and_then(|unit| {
                    unit.as_str()
                        .and_then(|s| TemperatureScale::from_str(s).map(Into::into).ok())
                })
                .or_else(|| quirk.and_then(|q| q.platform_temperature_sensor_units))
                .unwrap_or(TemperatureUnits::Celsius);

            log::debug!("units are reported as {units:?}");

            let value = match cap
                .state
                .pointer("/value/targetTemperature")
                .and_then(|v| v.as_f64())
                .map(|v| TemperatureValue::new(v, units))
            {
                Some(v) => {
                    let pref_units = self.state.get_temperature_scale().await;
                    log::debug!("reported temp is {v}, pref_units: {pref_units}");
                    let value = v.as_unit(pref_units.into()).value();
                    format!("{value:.2}")
                }
                None => "".to_string(),
            };

            log::debug!("setting value to {value}");

            return self.number.notify_state(client, &value).await;
        }

        Ok(())
    }
}

#[derive(Deserialize)]
pub struct IdInstAndUnits {
    id: String,
    instance: String,
    units: String,
}

pub async fn mqtt_set_temperature(
    Payload(value): Payload<String>,
    Params(IdInstAndUnits {
        id,
        instance,
        units,
    }): Params<IdInstAndUnits>,
    State(state): State<StateHandle>,
) -> anyhow::Result<()> {
    log::info!("Command: set-temperature for {id}: {value}");
    let device = state.resolve_device_for_control(&id).await?;

    let scale: TemperatureScale = units.parse()?;
    let target_value = TemperatureValue::parse_with_optional_scale(&value, Some(scale))?;

    state
        .device_set_target_temperature(&device, &instance, target_value)
        .await?;

    Ok(())
}
