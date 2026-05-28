//! The error type produced by every public entry point in this crate.
//!
//! Variants are organised by failure category, not by call site, so adding a
//! new internal failure mode does not necessarily mean adding a new variant.
//! Categories were chosen so a consumer can branch on what to do next:
//! `is_retryable` covers transient transport/IO faults, `is_config` covers
//! things only a human operator can fix, and `Unsupported` lets a caller
//! gracefully skip an operation a particular device does not implement.
//!
//! Inside the crate, helpers may use `anyhow` while assembling an operation;
//! the conversion to `GoveeApiError` happens at the public boundary so every
//! categorisation is deliberate. There is no `From<anyhow::Error>` impl for
//! that reason.

use std::error::Error as StdError;
use thiserror::Error;

/// Boxed underlying cause, used by variants that wrap a third-party error
/// type whose detail is worth preserving in the source chain.
pub type BoxedSource = Box<dyn StdError + Send + Sync + 'static>;

#[derive(Debug, Error)]
pub enum GoveeApiError {
    /// Transport-level failure: HTTP request, TCP connect, TLS, UDP send.
    /// Usually transient; `is_retryable` returns true.
    #[error("network: {0}")]
    Network(#[source] BoxedSource),

    /// The remote API responded but reported failure: either a non-2xx HTTP
    /// status or a 2xx body carrying an embedded `status`/`code` indicating
    /// failure. The full body is preserved in the message for diagnosis.
    #[error("api: {0}")]
    Api(String),

    /// A wire payload could not be parsed, or did not match the shape this
    /// crate expects. Covers JSON deserialisation, BLE frame decode, missing
    /// or wrong-typed fields. Indicates a wire-format change or a device or
    /// SKU this crate does not fully model.
    #[error("protocol: {0}")]
    Protocol(String),

    /// Credentials are missing, rejected, or expired. Not retryable without
    /// operator intervention.
    #[error("auth: {0}")]
    Auth(String),

    /// Configuration is missing or invalid: an env var is unset, has the
    /// wrong format, or points at an unwritable path. Not retryable without
    /// operator intervention.
    #[error("config: {0}")]
    Config(String),

    /// Local I/O failed: sqlite cache, PFX/PEM parsing, file read.
    #[error("io: {0}")]
    Io(#[source] BoxedSource),

    /// A BLE adapter, peripheral, or GATT operation failed.
    #[error("ble: {0}")]
    Ble(String),

    /// A LAN-API operation failed: UDP socket, scan timeout, listener task
    /// shutdown. Kept separate from `Network` because LAN failures usually
    /// mean a single device is offline rather than a broader outage, and a
    /// caller may choose to ignore them while still surfacing other transport
    /// errors.
    #[error("lan: {0}")]
    Lan(String),

    /// The requested operation is not available for this device, scene, or
    /// command. Covers both "device has no X capability" and "this codec
    /// does not implement decode." Not retryable; a UI may want to disable
    /// or hide the affected control.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl GoveeApiError {
    /// True if the failure is one a backoff-and-retry loop has a reasonable
    /// chance of recovering from without user intervention. Covers transport
    /// and local-IO categories.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Lan(_) | Self::Io(_))
    }

    /// True if the failure can only be fixed by an operator: bad credentials,
    /// missing configuration, or an operation the target device does not
    /// support. A retry loop should give up on these.
    pub fn is_config(&self) -> bool {
        matches!(self, Self::Auth(_) | Self::Config(_) | Self::Unsupported(_))
    }
}

pub type Result<T> = std::result::Result<T, GoveeApiError>;
