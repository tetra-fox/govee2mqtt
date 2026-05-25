use crate::hass_mqtt::base::{Device, EntityConfig, Origin};
use crate::hass_mqtt::instance::{EntityInstance, publish_entity_config};
use crate::hass_mqtt::topic::Topics;
use crate::service::device::Device as ServiceDevice;
use crate::service::hass::{HassClient, camel_case_to_space_separated, topic_safe_string};
use crate::service::state::StateHandle;
use async_trait::async_trait;
use govee_api::platform_api::DeviceCapability;
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct ButtonConfig {
    #[serde(flatten)]
    pub base: EntityConfig,

    pub command_topic: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_press: Option<String>,
}

impl ButtonConfig {
    #[allow(dead_code)]
    pub async fn for_device(
        topics: &Topics,
        device: &ServiceDevice,
        instance: &DeviceCapability,
    ) -> anyhow::Result<Self> {
        let command_topic = topics.switch_command(device, &instance.instance);
        let availability_topic = topics.availability();
        let unique_id = topics.entity_id(device, &instance.instance);

        Ok(Self {
            base: EntityConfig {
                availability_topic,
                name: Some(camel_case_to_space_separated(&instance.instance)),
                device_class: None,
                origin: Origin::default(),
                device: Device::for_device(topics, device),
                unique_id,
                entity_category: None,
                icon: None,
            },
            command_topic,
            payload_press: None,
        })
    }

    pub fn new<NAME: Into<String>, TOPIC: Into<String>>(
        topics: &Topics,
        name: NAME,
        topic: TOPIC,
    ) -> Self {
        let name = name.into();
        let unique_id = format!("global-{}", topic_safe_string(&name));
        Self {
            base: EntityConfig {
                availability_topic: topics.availability(),
                name: Some(name.to_string()),
                entity_category: None,
                origin: Origin::default(),
                device: Device::this_service(topics),
                unique_id: unique_id.clone(),
                device_class: None,
                icon: None,
            },
            command_topic: topic.into(),
            payload_press: None,
        }
    }

    pub fn activate_work_mode_preset(
        topics: &Topics,
        device: &ServiceDevice,
        name: &str,
        mode_name: &str,
        mode_num: i64,
        value: i64,
    ) -> Self {
        let unique_id = topics.entity_id(
            device,
            &format!(
                "preset-{mode}-{mode_num}-{value}",
                mode = topic_safe_string(mode_name)
            ),
        );
        let command_topic = topics.number_command(device, mode_name, mode_num);
        Self {
            base: EntityConfig {
                availability_topic: topics.availability(),
                name: Some(name.to_string()),
                entity_category: None,
                origin: Origin::default(),
                device: Device::for_device(topics, device),
                unique_id: unique_id.clone(),
                device_class: None,
                icon: None,
            },
            command_topic,
            payload_press: Some(value.to_string()),
        }
    }

    pub fn request_platform_data_for_device(topics: &Topics, device: &ServiceDevice) -> Self {
        let unique_id = topics.entity_id(device, "request-platform-data");
        let command_topic = topics.request_platform_data(device);
        Self {
            base: EntityConfig {
                availability_topic: topics.availability(),
                name: Some("Request Platform API State".to_string()),
                entity_category: Some("diagnostic".to_string()),
                origin: Origin::default(),
                device: Device::for_device(topics, device),
                unique_id: unique_id.clone(),
                device_class: None,
                icon: None,
            },
            command_topic,
            payload_press: None,
        }
    }
}

#[async_trait]
impl EntityInstance for ButtonConfig {
    async fn publish_config(&self, state: &StateHandle, client: &HassClient) -> anyhow::Result<()> {
        publish_entity_config("button", state, client, &self.base, self).await
    }

    async fn notify_state(&self, _client: &HassClient) -> anyhow::Result<()> {
        // Buttons have no state
        Ok(())
    }
}
