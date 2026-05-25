use crate::hass_mqtt::base::{DeviceDiscovery, EntityConfig};
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::HassClient;
use crate::service::state::{PublishedComponents, StateHandle};
use anyhow::Context;
use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Value as JsonValue, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[async_trait]
pub trait EntityInstance: Send + Sync {
    /// The discovery component for this entity: the platform it registers under
    /// and its config payload. The payload still carries `device`, `origin` and
    /// `availability`; [`EntityList::publish_config`] hoists those to the device
    /// level and strips them from the per-component fragment.
    fn component(&self) -> Component;

    /// Report current state. `device` is the entity's device, resolved once by
    /// the caller (see [`EntityList::notify_state`]) and passed in; it is None
    /// only for global entities that have no device.
    async fn notify_state(
        &self,
        device: Option<&ServiceDevice>,
        client: &HassClient,
    ) -> anyhow::Result<()>;

    /// The id of the device this entity reports state for, or None for global
    /// entities (version diagnostic, scenes). Used by [`EntityList::notify_state`]
    /// to resolve each device once instead of having every entity re-fetch it.
    fn device_id(&self) -> Option<&str> {
        None
    }
}

/// One entity's contribution to a device-discovery payload.
pub struct Component {
    pub platform: &'static str,
    /// The structured base, source of the device/origin/availability blocks
    /// hoisted to the device level and of the object id that groups components.
    pub base: EntityConfig,
    /// The full config serialized to json. The hoisted keys are stripped from
    /// this when it goes into the components map.
    pub config: JsonValue,
}

/// Build a discovery component from an entity config. `config` must be the
/// struct that embeds `base` via `#[serde(flatten)]`, so its serialized form
/// carries the same fields plus the entity-specific ones. The platform string
/// is the one each entity passed to the old per-entity publish path.
pub fn component<T: Serialize>(platform: &'static str, base: &EntityConfig, config: &T) -> Component {
    Component {
        platform,
        base: base.clone(),
        config: serde_json::to_value(config).expect("entity config serializes to json"),
    }
}

#[derive(Default, Clone)]
pub struct EntityList {
    entities: Vec<Arc<dyn EntityInstance + Send + Sync + 'static>>,
}

impl EntityList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add<E: EntityInstance + Send + Sync + 'static>(&mut self, e: E) {
        self.entities.push(Arc::new(e));
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Group every entity by its HA device and publish one device-discovery
    /// message per device. The device, origin and availability blocks are
    /// hoisted out of the components (they are identical within a device by
    /// construction) and written once at the device level.
    ///
    /// `previous` is the component map from the last pass. For a device that is
    /// still present but dropped a component, the dropped component is included
    /// in the republished payload as a tombstone (`{"p": platform}`), which is
    /// home assistant's signal to remove just that component. Devices that went
    /// away entirely are handled by the caller, which clears their whole topic.
    ///
    /// Returns the component map published this pass, keyed by device topic.
    pub async fn publish_config(
        &self,
        state: &StateHandle,
        client: &HassClient,
        previous: &PublishedComponents,
    ) -> anyhow::Result<PublishedComponents> {
        let disco = state.get_hass_disco_prefix().await;

        // Preserve enumeration order of devices while grouping their
        // components, so a device's discovery message is emitted near where its
        // entities were enumerated.
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, DeviceGroup> = HashMap::new();

        for e in &self.entities {
            let component = e.component();
            let object_id = component.base.device_object_id().to_string();
            let group = groups.entry(object_id.clone()).or_insert_with(|| {
                order.push(object_id.clone());
                DeviceGroup::from_first(&component)
            });
            group.add(component);
        }

        let mut published = PublishedComponents::new();
        let delay = tokio::time::Duration::from_millis(100);
        for object_id in order {
            let mut group = groups.remove(&object_id).expect("group exists");
            let topic = format!("{disco}/device/{object_id}/config");

            // Record what we're actually producing for this device, before
            // tombstones are mixed in, so the next pass diffs against the live
            // set rather than the removed ones.
            published.insert(topic.clone(), group.live_components());

            // Tombstone any component this device carried last time but no
            // longer produces.
            if let Some(prev) = previous.get(&topic) {
                for (unique_id, platform) in prev {
                    if !group.payload.components.contains_key(unique_id) {
                        group.tombstone(unique_id.clone(), platform.clone());
                    }
                }
            }

            client.publish_config(topic, group.into_payload()).await?;
            tokio::time::sleep(delay).await;
        }
        Ok(published)
    }

    pub async fn notify_state(
        &self,
        state: &StateHandle,
        client: &HassClient,
    ) -> anyhow::Result<()> {
        // Resolve each device once per pass and hand it to its entities, rather
        // than letting every entity re-lock shared state and clone the device.
        let mut resolved: HashMap<String, Option<ServiceDevice>> = HashMap::new();
        for e in &self.entities {
            let device = match e.device_id() {
                Some(id) => {
                    if !resolved.contains_key(id) {
                        resolved.insert(id.to_string(), state.device_by_id(id).await);
                    }
                    resolved[id].as_ref()
                }
                None => None,
            };
            e.notify_state(device, client)
                .await
                .context("EntityList::notify_state")?;
        }
        Ok(())
    }
}

/// Accumulates the components for one device, with the device/origin/
/// availability blocks taken from the first component (they are identical
/// across a device's components by construction) and hoisted to the top level.
struct DeviceGroup {
    payload: DeviceDiscovery,
    /// The platform of each live component by unique id, kept so the next pass
    /// can be told what was published and so a vanished component can be
    /// tombstoned with the right platform.
    platforms: HashMap<String, &'static str>,
}

impl DeviceGroup {
    fn from_first(component: &Component) -> Self {
        let base = &component.base;
        Self {
            payload: DeviceDiscovery {
                device: base.device.clone(),
                origin: base.origin.clone(),
                availability: base.availability.clone(),
                availability_mode: base.availability_mode,
                components: BTreeMap::new(),
            },
            platforms: HashMap::new(),
        }
    }

    fn add(&mut self, component: Component) {
        let Component {
            platform,
            base,
            mut config,
        } = component;

        // Hoisted to the device level; drop from the component so each fragment
        // doesn't repeat the shared device/origin/availability blocks.
        if let Some(obj) = config.as_object_mut() {
            obj.remove("device");
            obj.remove("origin");
            obj.remove("availability");
            obj.remove("availability_mode");
            obj.insert("platform".to_string(), JsonValue::from(platform));
        }

        self.platforms.insert(base.unique_id.clone(), platform);
        self.payload.components.insert(base.unique_id, config);
    }

    /// The live (non-tombstone) components keyed by unique id, with their
    /// platform. Call before adding tombstones.
    fn live_components(&self) -> HashMap<String, String> {
        self.platforms
            .iter()
            .map(|(id, platform)| (id.clone(), platform.to_string()))
            .collect()
    }

    /// Add a removal marker for a component that is no longer produced. Home
    /// assistant drops a component whose config is just the platform key.
    fn tombstone(&mut self, unique_id: String, platform: String) {
        self.payload
            .components
            .insert(unique_id, json!({ "platform": platform }));
    }

    fn into_payload(self) -> DeviceDiscovery {
        self.payload
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hass_mqtt::base::{Availability, Device, Origin};

    fn base(unique_id: &str, device_id: &str) -> EntityConfig {
        EntityConfig {
            availability: vec![Availability {
                topic: "g/availability".to_string(),
            }],
            availability_mode: Some("all"),
            name: Some(unique_id.to_string()),
            device_class: None,
            origin: Origin::default(),
            device: Device {
                name: "Dev".to_string(),
                identifiers: vec![device_id.to_string()],
                ..Default::default()
            },
            unique_id: unique_id.to_string(),
            entity_category: None,
            icon: None,
        }
    }

    #[derive(serde::Serialize)]
    struct Flat {
        #[serde(flatten)]
        base: EntityConfig,
        command_topic: String,
    }

    fn switch_component(unique_id: &str, device_id: &str) -> Component {
        let base = base(unique_id, device_id);
        let flat = Flat {
            base: base.clone(),
            command_topic: format!("g/{unique_id}/cmd"),
        };
        component("switch", &base, &flat)
    }

    #[test]
    fn hoists_shared_blocks_and_keys_components_by_unique_id() {
        let mut group = DeviceGroup::from_first(&switch_component("a", "dev1"));
        group.add(switch_component("a", "dev1"));
        group.add(switch_component("b", "dev1"));

        let value = serde_json::to_value(group.into_payload()).unwrap();

        // shared device/origin/availability are at the top level
        assert_eq!(value["dev"]["identifiers"][0], "dev1");
        assert_eq!(value["o"]["name"], "govee2mqtt");
        assert_eq!(value["avty"][0]["t"].as_str(), None); // we use long key
        assert_eq!(value["avty"][0]["topic"], "g/availability");
        assert_eq!(value["availability_mode"], "all");

        // each component carries its platform and own fields, but not the
        // hoisted blocks
        let a = &value["cmps"]["a"];
        assert_eq!(a["platform"], "switch");
        assert_eq!(a["command_topic"], "g/a/cmd");
        assert!(a.get("device").is_none());
        assert!(a.get("origin").is_none());
        assert!(a.get("availability").is_none());
        assert!(value["cmps"].get("b").is_some());
    }

    #[test]
    fn live_components_reports_platforms_before_tombstones() {
        let mut group = DeviceGroup::from_first(&switch_component("a", "dev1"));
        group.add(switch_component("a", "dev1"));

        let live = group.live_components();
        assert_eq!(live.get("a").map(String::as_str), Some("switch"));

        group.tombstone("gone".to_string(), "sensor".to_string());
        // live_components captured before the tombstone doesn't include it
        assert!(!live.contains_key("gone"));
    }

    #[test]
    fn tombstone_is_platform_only() {
        let mut group = DeviceGroup::from_first(&switch_component("a", "dev1"));
        group.add(switch_component("a", "dev1"));
        group.tombstone("gone".to_string(), "sensor".to_string());

        let value = serde_json::to_value(group.into_payload()).unwrap();
        let gone = &value["cmps"]["gone"];
        assert_eq!(gone["platform"], "sensor");
        // a removal marker carries only the platform key
        assert_eq!(gone.as_object().unwrap().len(), 1);
    }
}
