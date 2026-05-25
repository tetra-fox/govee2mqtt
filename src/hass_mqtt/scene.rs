use crate::hass_mqtt::base::EntityConfig;
use crate::hass_mqtt::instance::{Component, EntityInstance, component};
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::HassClient;
use async_trait::async_trait;
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct SceneConfig {
    #[serde(flatten)]
    pub base: EntityConfig,

    pub command_topic: String,
    pub payload_on: String,
}

#[async_trait]
impl EntityInstance for SceneConfig {
    fn component(&self) -> Component {
        component("scene", &self.base, self)
    }

    async fn notify_state(
        &self,
        _device: Option<&ServiceDevice>,
        _client: &HassClient,
    ) -> anyhow::Result<()> {
        // Scenes have no state
        Ok(())
    }
}
