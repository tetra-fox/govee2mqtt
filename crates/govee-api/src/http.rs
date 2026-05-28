//! Shared HTTP response helpers used by both the platform API and the
//! undocumented app API clients. These handle the two failure shapes Govee uses:
//! a non-2xx HTTP status, and a 2xx response whose JSON body carries an embedded
//! `status`/`code` field indicating failure.

use crate::error::{ApiResult, GoveeApiError};
use serde::Deserialize;
use thiserror::Error;

/// The shape of a non-2xx HTTP response from a Govee endpoint. Preserved as
/// a typed value for external consumers; the in-tree HTTP helpers now report
/// failures via `GoveeApiError::Api` directly and never construct this type.
#[derive(Error, Debug)]
#[error("Failed with status {status} {}: {content}", .status.canonical_reason().unwrap_or(""))]
#[allow(dead_code)]
pub struct HttpRequestFailed {
    status: reqwest::StatusCode,
    content: String,
}

pub fn from_json<T: serde::de::DeserializeOwned, S: AsRef<[u8]>>(text: S) -> ApiResult<T> {
    let text = text.as_ref();
    serde_json_path_to_error::from_slice(text).map_err(|err| {
        GoveeApiError::Protocol(format!(
            "{} {err}. Input: {}",
            std::any::type_name::<T>(),
            String::from_utf8_lossy(text)
        ))
    })
}

#[derive(Deserialize, Debug)]
struct EmbeddedRequestStatus {
    #[serde(alias = "msg")]
    message: String,
    #[serde(alias = "code")]
    status: u16,
}

pub async fn json_body<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> ApiResult<T> {
    let url = response.url().clone();
    let data = response.bytes().await.map_err(|err| {
        GoveeApiError::Network(format!("reading {url} response body: {err}").into())
    })?;

    if let Ok(status) = from_json::<EmbeddedRequestStatus, _>(&data)
        && status.status != reqwest::StatusCode::OK.as_u16()
    {
        return Err(GoveeApiError::Api(format!(
            "request to {url} failed with status={status_code} {message}. \
             Full response: {body}",
            status_code = status.status,
            message = status.message,
            body = String::from_utf8_lossy(&data),
        )));
    }

    from_json(&data)
}

pub async fn http_response_body<R: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> ApiResult<R> {
    let url = response.url().clone();

    let status = response.status();
    if !status.is_success() {
        let body_bytes = response.bytes().await.map_err(|err| {
            GoveeApiError::Network(
                format!(
                    "request {url} status {} {}, and failed to read response body: {err}",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or(""),
                )
                .into(),
            )
        })?;

        return Err(GoveeApiError::Api(format!(
            "request {url} status {} {}. Response body: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            String::from_utf8_lossy(&body_bytes),
        )));
    }
    json_body(response).await
}
