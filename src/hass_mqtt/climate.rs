use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::instance::{Component, EntityInstance};
use crate::hass_mqtt::number::NumberConfig;
use crate::hass_mqtt::router::{Params, Payload, State};
use crate::hass_mqtt::topic::Topics;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::{HassClient, topic_safe_id, topic_safe_string};
use crate::service::state::StateHandle;
use govee_api::platform_api::{DeviceCapability, parse_temperature_constraints};
use govee_api::temperature::{
    DEVICE_CLASS_TEMPERATURE, TemperatureScale, TemperatureUnits, TemperatureValue,
};
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
        topics: &Topics,
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
        let command_topic = topics.set_temperature(device, &instance.instance, &units.to_string());
        let state_topic = topics.advise_set_temperature(device);
        let (availability, availability_mode) = EntityConfig::device_availability(topics, device);

        Ok(Self {
            number: NumberConfig {
                base: EntityConfig {
                    availability,
                    availability_mode,
                    name: Some(name),
                    entity_category: None,
                    origin: Origin::default(),
                    device: Device::for_device(topics, device),
                    unique_id: unique_id.clone(),
                    device_class: Some(DEVICE_CLASS_TEMPERATURE),
                    icon: Some("mdi:thermometer".to_string()),
                },
                state_topic: Some(state_topic),
                command_topic,
                min: Some(constraints.min.value().floor() as f32),
                max: Some(constraints.max.value().ceil() as f32),
                step: 1.0,
                unit_of_measurement: Some(units.unit_of_measurement().to_string()),
            },
            device_id: device.id.to_string(),
            state: state.clone(),
            instance_name: instance.instance.to_string(),
        })
    }
}

#[async_trait::async_trait]
impl EntityInstance for TargetTemperatureEntity {
    fn component(&self) -> Component {
        self.number.component()
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

        log::trace!("notify_state for {device} {}", self.instance_name);

        if let Some(cap) = device.get_state_capability_by_instance(&self.instance_name) {
            log::trace!("have: {cap:?}");

            let units = cap
                .state
                .pointer("/value/unit")
                .and_then(|unit| {
                    unit.as_str()
                        .and_then(|s| TemperatureScale::from_str(s).map(Into::into).ok())
                })
                .or_else(|| quirk.and_then(|q| q.platform_temperature_sensor_units))
                .unwrap_or(TemperatureUnits::Celsius);

            log::trace!("units are reported as {units:?}");

            let value = match cap
                .state
                .pointer("/value/targetTemperature")
                .and_then(|v| v.as_f64())
                .map(|v| TemperatureValue::new(v, units))
            {
                Some(v) => {
                    let pref_units = self.state.get_temperature_scale().await;
                    log::trace!("reported temp is {v}, pref_units: {pref_units}");
                    let value = v.as_unit(pref_units.into()).value();
                    format!("{value:.2}")
                }
                None => "".to_string(),
            };

            log::trace!("setting value to {value}");

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
