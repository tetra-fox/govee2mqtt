// This module implements the Govee Platform API V1 as described at:
// <https://developer.govee.com/reference/get-you-devices>
//
// It is NOT the same thing as the older, but confusingly versioned
// with a higher number, Govee HTTP API v2 that is described at
// <https://govee.readme.io/reference/getlightdeviceinfo>

use crate::http::http_response_body;
use crate::opt_env_var;
use crate::temperature::{
    TemperatureConstraints, TemperatureScale, TemperatureUnits, TemperatureValue,
};
use anyhow::anyhow;
use reqwest::Method;
use std::str::FromStr;
use std::time::Duration;
use uuid::Uuid;

mod client;
mod wire;

// The capability type model (DeviceCapability, DeviceParameters, DeviceType, ...)
// lives in crate::model; the shared HTTP helpers in crate::http. Re-export the
// pieces callers reach for via `platform_api::` so existing import paths keep
// working.
pub use crate::http::{HttpRequestFailed, json_body};
pub use crate::model::*;
pub use wire::{
    ControlDeviceResponseCapability, DeviceCapabilityState, HttpDeviceInfo, HttpDeviceState,
};

const SERVER: &str = "https://openapi.api.govee.com";
pub const ONE_WEEK: Duration = Duration::from_secs(86400 * 7);
pub const FIVE_MINUTES: Duration = Duration::from_secs(5 * 60);

fn endpoint(url: &str) -> String {
    format!("{SERVER}{url}")
}

fn new_request_id() -> String {
    Uuid::new_v4().to_string()
}

#[derive(clap::Parser, Debug)]
pub struct GoveeApiArguments {
    /// The Govee API Key. If not passed here, it will be read from
    /// the GOVEE2MQTT_API_KEY environment variable.
    #[arg(long, global = true)]
    pub api_key: Option<String>,
}

impl GoveeApiArguments {
    pub fn opt_api_key(&self) -> anyhow::Result<Option<String>> {
        match &self.api_key {
            Some(key) => Ok(Some(key.to_string())),
            None => opt_env_var("GOVEE2MQTT_API_KEY"),
        }
    }

    pub fn api_key(&self) -> anyhow::Result<String> {
        self.opt_api_key()?.ok_or_else(|| {
            anyhow::anyhow!(
                "Please specify the api key either via the \
                --api-key parameter or by setting $GOVEE2MQTT_API_KEY"
            )
        })
    }

    pub fn api_client(&self) -> anyhow::Result<GoveeApiClient> {
        let key = self.api_key()?;
        Ok(GoveeApiClient::new(key))
    }
}

#[derive(Clone)]
pub struct GoveeApiClient {
    key: String,
}

impl GoveeApiClient {
    pub fn new(key: String) -> Self {
        Self { key }
    }

    async fn get_request_with_json_response<T: reqwest::IntoUrl, R: serde::de::DeserializeOwned>(
        &self,
        url: T,
    ) -> anyhow::Result<R> {
        let response = crate::http_client()
            .request(Method::GET, url)
            .timeout(Duration::from_secs(60))
            .header("Govee-API-Key", &self.key)
            .send()
            .await?;

        http_response_body(response).await
    }

    async fn request_with_json_response<
        T: reqwest::IntoUrl,
        B: serde::Serialize,
        R: serde::de::DeserializeOwned,
    >(
        &self,
        method: Method,
        url: T,
        body: &B,
    ) -> anyhow::Result<R> {
        let response = crate::http_client()
            .request(method, url)
            .timeout(Duration::from_secs(60))
            .header("Govee-API-Key", &self.key)
            .json(body)
            .send()
            .await?;

        http_response_body(response).await
    }
}

pub fn sort_and_dedup_scenes(mut scenes: Vec<String>) -> Vec<String> {
    scenes.sort_by_key(|s| s.to_ascii_lowercase());
    scenes.dedup();
    scenes
}

pub fn parse_temperature_constraints(
    instance: &DeviceCapability,
) -> anyhow::Result<TemperatureConstraints> {
    let units = instance
        .struct_field_by_name("unit")
        .and_then(|field| {
            field.default_value.as_ref().and_then(|v| {
                v.as_str()
                    .and_then(|s| TemperatureScale::from_str(s).map(Into::into).ok())
            })
        })
        .unwrap_or(TemperatureUnits::Fahrenheit);

    let temperature = instance
        .struct_field_by_name("temperature")
        .ok_or_else(|| anyhow!("no temperature field in {instance:?}"))?;
    match &temperature.field_type {
        DeviceParameters::Integer { unit, range } => {
            let range_units = unit
                .as_deref()
                .and_then(|s| TemperatureScale::from_str(s).map(Into::into).ok())
                .unwrap_or(units);

            let min = TemperatureValue::new(range.min.into(), range_units);
            let max = TemperatureValue::new(range.max.into(), range_units);

            Ok(TemperatureConstraints {
                min: min.as_unit(units),
                max: max.as_unit(units),
            })
        }
        _ => {
            anyhow::bail!("Unexpected temperature value in {instance:?}");
        }
    }
}
