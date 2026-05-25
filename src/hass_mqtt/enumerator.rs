use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::button::ButtonConfig;
use crate::hass_mqtt::climate::TargetTemperatureEntity;
use crate::hass_mqtt::humidifier::Humidifier;
use crate::hass_mqtt::instance::EntityList;
use crate::hass_mqtt::light::DeviceLight;
use crate::hass_mqtt::number::{CapabilityNumber, MusicSensitivityNumber, WorkModeNumber};
use crate::hass_mqtt::scene::SceneConfig;
use crate::hass_mqtt::select::{CapabilityModeSelect, SceneModeSelect, WorkModeSelect};
use crate::hass_mqtt::sensor::{
    CapabilityEventSensor, CapabilitySensor, DeviceStatusDiagnostic, GlobalFixedDiagnostic,
};
use crate::hass_mqtt::switch::{CapabilitySwitch, MusicAutoColorSwitch, OutletSwitch, PowerSwitch};
use crate::hass_mqtt::work_mode::ParsedWorkMode;
use crate::service::device::Device as ServiceDevice;
use crate::service::state::StateHandle;
use crate::version_info::govee_version;
use anyhow::Context;
use govee_api::platform_api::{DeviceCapability, DeviceCapabilityKind, DeviceType};

use uuid::Uuid;

/// The result of enumerating every entity for registration. `complete` is false
/// when a source we depend on failed (eg: the undoc one-click API), meaning the
/// entity list is partial. Callers must not garbage-collect stale discovery
/// configs from a partial pass, or a transient/permanent source failure would
/// delete entities that still exist.
pub struct Enumeration {
    pub entities: EntityList,
    pub complete: bool,
}

pub async fn enumerate_all_entites(state: &StateHandle) -> anyhow::Result<Enumeration> {
    let mut entities = EntityList::new();
    let mut complete = true;

    enumerate_global_entities(state, &mut entities).await?;
    complete &= enumerate_scenes(state, &mut entities).await?;

    let devices = state.devices().await;

    for d in &devices {
        enumerate_entities_for_device(d, state, &mut entities)
            .await
            .with_context(|| format!("Config::for_device({d})"))?;
    }

    Ok(Enumeration { entities, complete })
}

async fn enumerate_global_entities(
    state: &StateHandle,
    entities: &mut EntityList,
) -> anyhow::Result<()> {
    let topics = state.topics().await;
    entities.add(GlobalFixedDiagnostic::new(
        &topics,
        "Version",
        govee_version(),
    ));
    entities.add(ButtonConfig::new(
        &topics,
        "Purge Caches",
        topics.purge_caches(),
    ));
    Ok(())
}

/// Returns false if the one-click list couldn't be fetched/parsed, so the
/// caller knows the scene entities are missing from this pass and must not GC
/// them. A failure here is non-fatal to registration: every other entity still
/// registers, the existing scene configs are left alone, and the next
/// successful pass restores them.
async fn enumerate_scenes(state: &StateHandle, entities: &mut EntityList) -> anyhow::Result<bool> {
    let Some(undoc) = state.get_undoc_client().await else {
        return Ok(true);
    };
    let topics = state.topics().await;
    let items = match undoc.parse_one_clicks().await {
        Ok(items) => items,
        Err(err) => {
            log::warn!("Failed to parse one-clicks, leaving scene entities as-is: {err:#}");
            return Ok(false);
        }
    };
    for oc in items {
        let unique_id =
            topics.one_click_id(Uuid::new_v5(&Uuid::NAMESPACE_DNS, oc.name.as_bytes()).simple());
        let (availability, availability_mode) = EntityConfig::global_availability(&topics);
        entities.add(SceneConfig {
            base: EntityConfig {
                availability,
                availability_mode,
                name: Some(oc.name.to_string()),
                entity_category: None,
                origin: Origin::default(),
                device: Device::this_service(&topics),
                unique_id: unique_id.clone(),
                device_class: None,
                icon: None,
            },
            command_topic: topics.oneclick(),
            payload_on: oc.name,
        });
    }

    Ok(true)
}

async fn entities_for_work_mode(
    d: &ServiceDevice,
    state: &StateHandle,
    cap: &DeviceCapability,
    entities: &mut EntityList,
) -> anyhow::Result<()> {
    let topics = state.topics().await;
    let mut work_modes = ParsedWorkMode::with_capability(cap)?;
    work_modes.adjust_for_device(&d.sku);

    let quirk = d.resolve_quirk();

    for work_mode in work_modes.modes.values() {
        let Some(mode_num) = work_mode.value.as_i64() else {
            continue;
        };

        let range = work_mode.contiguous_value_range();

        let show_as_preset = work_mode.should_show_as_preset()
            || quirk
                .as_ref()
                .map(|q| q.should_show_mode_as_preset(&work_mode.name))
                .unwrap_or(false);

        if show_as_preset {
            if work_mode.values.is_empty() {
                entities.add(ButtonConfig::activate_work_mode_preset(
                    &topics,
                    d,
                    &format!("Activate Mode: {}", work_mode.label()),
                    &work_mode.name,
                    mode_num,
                    work_mode.default_value(),
                ));
            } else {
                for value in &work_mode.values {
                    if let Some(mode_value) = value.value.as_i64() {
                        entities.add(ButtonConfig::activate_work_mode_preset(
                            &topics,
                            d,
                            &value.computed_label,
                            &work_mode.name,
                            mode_num,
                            mode_value,
                        ));
                    }
                }
            }
        } else {
            let label = work_mode.label().to_string();

            entities.add(WorkModeNumber::new(
                &topics,
                d,
                label,
                &work_mode.name,
                work_mode.value.clone(),
                range,
            ));
        }
    }

    entities.add(WorkModeSelect::new(&topics, d, &work_modes));

    Ok(())
}

pub async fn enumerate_entities_for_device(
    d: &ServiceDevice,
    state: &StateHandle,
    entities: &mut EntityList,
) -> anyhow::Result<()> {
    if !d.is_controllable() {
        return Ok(());
    }

    let topics = state.topics().await;

    entities.add(DeviceStatusDiagnostic::new(&topics, d));
    entities.add(ButtonConfig::request_platform_data_for_device(&topics, d));

    let is_light_capable =
        d.supports_rgb() || d.get_color_temperature_range().is_some() || d.supports_brightness();
    let wants_scene_select = d.device_type() != DeviceType::Light;

    // The light entity's effect_list and the Mode/Scene select's options are the
    // same scene list; fetch it once and share it between them.
    let scenes = if is_light_capable || wants_scene_select {
        state.device_list_scenes(d).await.unwrap_or_else(|err| {
            log::error!("Unable to list scenes for {d}: {err:#}");
            vec![]
        })
    } else {
        vec![]
    };

    if is_light_capable {
        entities.add(DeviceLight::for_device(&topics, d, None, &scenes));
    }

    if matches!(
        d.device_type(),
        DeviceType::Humidifier | DeviceType::Dehumidifier
    ) {
        entities.add(Humidifier::new(&topics, d, state).await?);
    }

    if wants_scene_select && let Some(select) = SceneModeSelect::new(&topics, d, &scenes) {
        entities.add(select);
    }

    // Multi-outlet sockets only expose a single combined powerSwitch via the
    // platform API, but the IoT status reports each outlet separately. Surface
    // each outlet as its own switch alongside that combined one. The read path
    // works today; per-outlet control is stubbed pending the full feature.
    // <https://github.com/wez/govee2mqtt/issues/65>
    if let Some(count) = d.socket_outlet_count() {
        for index in 0..count {
            entities.add(OutletSwitch::new(&topics, d, index));
        }
    }

    // A single plug/switch we know only from a quirk has no platform metadata,
    // so the capability loop below never runs and never creates a powerSwitch.
    // Synthesize one from the quirk. Multi-outlet sockets are covered above.
    if d.device_type() == DeviceType::Socket
        && d.socket_outlet_count().is_none()
        && d.http_device_info.is_none()
    {
        entities.add(PowerSwitch::new(&topics, d));
    }

    if let Some(info) = &d.http_device_info {
        for cap in &info.capabilities {
            match &cap.kind {
                DeviceCapabilityKind::Toggle | DeviceCapabilityKind::OnOff => {
                    entities.add(CapabilitySwitch::new(&topics, d, cap).await?);
                }
                // Color and scene capabilities are surfaced through the light
                // entity and the Mode/Scene select, not as their own entities.
                // The music mode itself is one of those scene options; its
                // sensitivity/auto-color preferences are added below.
                DeviceCapabilityKind::ColorSetting
                | DeviceCapabilityKind::SegmentColorSetting
                | DeviceCapabilityKind::MusicSetting
                | DeviceCapabilityKind::DynamicScene => {}

                // brightness is the light entity; humidity is the humidifier.
                DeviceCapabilityKind::Range if cap.instance == "brightness" => {}
                DeviceCapabilityKind::Range if cap.instance == "humidity" => {}
                DeviceCapabilityKind::Range => {
                    entities.add(CapabilityNumber::new(&topics, d, cap));
                }

                DeviceCapabilityKind::Mode => {
                    if let Some(select) = CapabilityModeSelect::new(&topics, d, cap) {
                        entities.add(select);
                    }
                }

                DeviceCapabilityKind::Event => {
                    entities.add(CapabilityEventSensor::new(&topics, d, cap));
                }

                DeviceCapabilityKind::WorkMode => {
                    entities_for_work_mode(d, state, cap, entities).await?;
                }

                DeviceCapabilityKind::Property => {
                    entities.add(CapabilitySensor::new(&topics, d, state, cap).await?);
                }

                DeviceCapabilityKind::TemperatureSetting => {
                    entities.add(TargetTemperatureEntity::new(&topics, d, state, cap).await?);
                }

                kind => {
                    log::warn!(
                        "Do something about {kind:?} {} for {d} {cap:?}",
                        cap.instance
                    );
                }
            }
        }

        // When the device has a music mode, the "Music: X" scenes send a
        // sensitivity and auto-color value alongside the mode. Expose those two
        // as adjustable preferences; Govee doesn't report them back so they only
        // take effect on the next music-scene selection.
        if info.capability_by_instance("musicMode").is_some() {
            entities.add(MusicSensitivityNumber::new(&topics, d));
            entities.add(MusicAutoColorSwitch::new(&topics, d));
        }

        if let Some(segments) = info.supports_segmented_rgb() {
            for n in segments {
                entities.add(DeviceLight::for_device(&topics, d, Some(n), &[]));
            }
        }
    }
    Ok(())
}
