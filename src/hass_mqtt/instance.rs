use crate::hass_mqtt::base::EntityConfig;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::HassClient;
use crate::service::state::StateHandle;
use anyhow::Context;
use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

#[async_trait]
pub trait EntityInstance: Send + Sync {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()>;

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

pub async fn publish_entity_config<T: Serialize>(
    integration: &str,
    state: &StateHandle,
    client: &HassClient,
    base: &EntityConfig,
    config: &T,
) -> anyhow::Result<()> {
    let disco = state.get_hass_disco_prefix().await;
    let topic = format!(
        "{disco}/{integration}/{unique_id}/config",
        unique_id = base.unique_id
    );

    // Record the topic so the next registration can remove this entity if it
    // is no longer produced (see HassClient::register_with_hass).
    state.record_published_config_topic(topic.clone()).await;

    client.publish_config(topic, config).await
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

    pub async fn publish_config(
        &self,
        state: &StateHandle,
        client: &HassClient,
    ) -> anyhow::Result<()> {
        // Allow HASS time to process each entity before registering the next
        let delay = tokio::time::Duration::from_millis(100);
        for e in &self.entities {
            e.publish_config(state, client)
                .await
                .context("EntityList::publish_config")?;
            tokio::time::sleep(delay).await;
        }
        Ok(())
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
