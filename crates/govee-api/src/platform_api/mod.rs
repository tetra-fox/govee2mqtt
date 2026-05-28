// This module implements the Govee Platform API V1 as described at:
// <https://developer.govee.com/reference/get-you-devices>
//
// It is NOT the same thing as the older, but confusingly versioned
// with a higher number, Govee HTTP API v2 that is described at
// <https://govee.readme.io/reference/getlightdeviceinfo>

use crate::error::{ApiResult, GoveeApiError};
use crate::http::http_response_body;
use crate::opt_env_var;
use crate::temperature::{
    TemperatureConstraints, TemperatureScale, TemperatureUnits, TemperatureValue,
};
use reqwest::Method;
use std::str::FromStr;
use std::time::Duration;
use uuid::Uuid;

fn network<E: std::fmt::Display>(context: &str) -> impl FnOnce(E) -> GoveeApiError + '_ {
    move |err| GoveeApiError::Network(format!("{context}: {err}").into())
}

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
    #[arg(long = "govee-api-key", global = true)]
    pub api_key: Option<String>,
}

impl GoveeApiArguments {
    pub fn opt_api_key(&self) -> ApiResult<Option<String>> {
        match &self.api_key {
            Some(key) => Ok(Some(key.to_string())),
            None => opt_env_var("GOVEE2MQTT_API_KEY"),
        }
    }

    pub fn api_key(&self) -> ApiResult<String> {
        self.opt_api_key()?.ok_or_else(|| {
            GoveeApiError::Auth(
                "specify the api key either via the \
                 --govee-api-key parameter or by setting $GOVEE2MQTT_API_KEY"
                    .into(),
            )
        })
    }

    pub fn api_client(&self) -> ApiResult<GoveeApiClient> {
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
    ) -> ApiResult<R> {
        let response = crate::http_client()
            .request(Method::GET, url)
            .timeout(Duration::from_secs(60))
            .header("Govee-API-Key", &self.key)
            .send()
            .await
            .map_err(network("platform GET"))?;

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
    ) -> ApiResult<R> {
        let response = crate::http_client()
            .request(method, url)
            .timeout(Duration::from_secs(60))
            .header("Govee-API-Key", &self.key)
            .json(body)
            .send()
            .await
            .map_err(network("platform request"))?;

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
) -> ApiResult<TemperatureConstraints> {
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
        .ok_or_else(|| GoveeApiError::Protocol(format!("no temperature field in {instance:?}")))?;
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
        _ => Err(GoveeApiError::Protocol(format!(
            "unexpected temperature value in {instance:?}"
        ))),
    }
}
