mod env;
pub use env::opt_env_var;

mod error;
pub use error::{BoxedSource, GoveeApiError, Result};

/// A process-wide reqwest client, built once. reqwest pools connections per
/// origin internally, so reusing one client keeps TLS sessions and keep-alive
/// connections warm across the many requests this crate makes to the same Govee
/// hosts. Per-request timeouts are set on the RequestBuilder, since they vary by
/// call site.
pub(crate) fn http_client() -> &'static reqwest::Client {
    use once_cell::sync::Lazy;
    static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
        reqwest::Client::builder()
            .build()
            .expect("build reqwest client")
    });
    Lazy::force(&CLIENT)
}

pub mod ble;
pub mod cache;
pub mod http;
pub mod lan_api;
pub mod model;
pub mod platform_api;
pub mod temperature;
pub mod undoc_api;
