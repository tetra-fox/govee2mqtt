//! Shared HTTP response helpers used by both the platform API and the
//! undocumented app API clients. These handle the two failure shapes Govee uses:
//! a non-2xx HTTP status, and a 2xx response whose JSON body carries an embedded
//! `status`/`code` field indicating failure.

use anyhow::Context;
use serde::Deserialize;
use thiserror::Error;

pub fn from_json<T: serde::de::DeserializeOwned, S: AsRef<[u8]>>(text: S) -> anyhow::Result<T> {
    let text = text.as_ref();
    serde_json_path_to_error::from_slice(text).map_err(|err| {
        anyhow::anyhow!(
            "{} {err}. Input: {}",
            std::any::type_name::<T>(),
            String::from_utf8_lossy(text)
        )
    })
}

#[derive(Deserialize, Debug)]
struct EmbeddedRequestStatus {
    #[serde(alias = "msg")]
    message: String,
    #[serde(alias = "code")]
    status: u16,
}

#[derive(Error, Debug)]
#[error("Failed with status {status} {}: {content}", .status.canonical_reason().unwrap_or(""))]
pub struct HttpRequestFailed {
    status: reqwest::StatusCode,
    content: String,
}

pub async fn json_body<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> anyhow::Result<T> {
    let url = response.url().clone();
    let data = response
        .bytes()
        .await
        .with_context(|| format!("read {url} response body"))?;

    if let Ok(status) = from_json::<EmbeddedRequestStatus, _>(&data)
        && status.status != reqwest::StatusCode::OK.as_u16()
    {
        if let Ok(code) = reqwest::StatusCode::from_u16(status.status) {
            return Err(HttpRequestFailed {
                status: code,
                content: format!(
                    "Request to {url} failed with code {code} {message}. Full response: {}",
                    String::from_utf8_lossy(&data),
                    message = status.message
                ),
            })
            .with_context(|| format!("parsing {url} response"));
        }

        anyhow::bail!(
            "Request to {url} failed with status={status} {message}. Full response was: {}",
            String::from_utf8_lossy(&data),
            status = status.status,
            message = status.message,
        );
    }

    from_json(&data).with_context(|| format!("parsing {url} response"))
}

pub async fn http_response_body<R: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> anyhow::Result<R> {
    let url = response.url().clone();

    let status = response.status();
    if !status.is_success() {
        let body_bytes = response.bytes().await.with_context(|| {
            format!(
                "request {url} status {}: {}, and failed to read response body",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            )
        })?;

        anyhow::bail!(
            "request {url} status {}: {}. Response body: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            String::from_utf8_lossy(&body_bytes)
        );
    }
    json_body(response).await.with_context(|| {
        format!(
            "request {url} status {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        )
    })
}
