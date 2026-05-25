use crate::cache::{CacheComputeResult, CacheGetOptions, cache_get};
use crate::http::http_response_body;
use crate::model::{DeviceCapability, DeviceCapabilityKind, DeviceParameters, EnumOption};
use crate::opt_env_var;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

mod wire;
pub use wire::*;

// <https://github.com/constructorfleet/homebridge-ultimate-govee/blob/main/src/data/clients/RestClient.ts>

const APP_VERSION: &str = "6.5.02";
const HALF_DAY: Duration = Duration::from_secs(3600 * 12);
const ONE_DAY: Duration = Duration::from_secs(86400);
const ONE_WEEK: Duration = Duration::from_secs(86400 * 7);
const FIFTEEN_MINS: Duration = Duration::from_secs(60 * 15);

fn user_agent() -> String {
    format!(
        "GoveeHome/{APP_VERSION} (com.ihoment.GoVeeSensor; build:2; iOS 16.5.0) Alamofire/5.6.4"
    )
}

fn epoch_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("unix epoch in the past")
        .as_millis()
}

pub fn ms_timestamp() -> String {
    epoch_millis().to_string()
}

#[derive(Clone, clap::Parser, Debug)]
pub struct UndocApiArguments {
    /// The email address you registered with Govee.
    /// If not passed here, it will be read from
    /// the GOVEE2MQTT_EMAIL environment variable.
    #[arg(long, global = true)]
    pub govee_email: Option<String>,

    /// The password for your Govee account.
    /// If not passed here, it will be read from
    /// the GOVEE2MQTT_PASSWORD environment variable.
    #[arg(long, global = true)]
    pub govee_password: Option<String>,

    /// Where to find the trust CA certificate used to verify the AWS IoT
    /// endpoint. Defaults to the system CA bundle, which includes the Amazon
    /// root CA that the IoT endpoint chains to.
    #[arg(
        long,
        global = true,
        default_value = "/etc/ssl/certs/ca-certificates.crt"
    )]
    pub amazon_root_ca: PathBuf,
}

impl UndocApiArguments {
    pub fn opt_email(&self) -> anyhow::Result<Option<String>> {
        match &self.govee_email {
            Some(key) => Ok(Some(key.to_string())),
            None => opt_env_var("GOVEE2MQTT_EMAIL"),
        }
    }

    pub fn email(&self) -> anyhow::Result<String> {
        self.opt_email()?.ok_or_else(|| {
            anyhow::anyhow!(
                "Please specify the govee account email either via the \
                --govee-email parameter or by setting $GOVEE2MQTT_EMAIL"
            )
        })
    }

    pub fn opt_password(&self) -> anyhow::Result<Option<String>> {
        match &self.govee_password {
            Some(key) => Ok(Some(key.to_string())),
            None => opt_env_var("GOVEE2MQTT_PASSWORD"),
        }
    }

    pub fn password(&self) -> anyhow::Result<String> {
        self.opt_password()?.ok_or_else(|| {
            anyhow::anyhow!(
                "Please specify the govee account password either via the \
                --govee-password parameter or by setting $GOVEE2MQTT_PASSWORD"
            )
        })
    }

    pub fn api_client(&self) -> anyhow::Result<GoveeUndocumentedApi> {
        let email = self.email()?;
        let password = self.password()?;
        Ok(GoveeUndocumentedApi::new(email, password))
    }
}

#[derive(Clone)]
pub struct GoveeUndocumentedApi {
    email: String,
    password: String,
    client_id: String,
}

impl GoveeUndocumentedApi {
    pub fn new(email: String, password: String) -> Self {
        let client_id = Uuid::new_v5(&Uuid::NAMESPACE_DNS, email.as_bytes());
        let client_id = format!("{}", client_id.simple());
        Self {
            email,
            password,
            client_id,
        }
    }

    /// Build a request to the app API (app2.govee.com) carrying the standard
    /// identification headers the app sends. `token`, when present, is the
    /// account Bearer token from `login_account`.
    fn app_request(
        &self,
        method: Method,
        url: impl reqwest::IntoUrl,
        token: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let mut req = crate::http_client()
            .request(method, url)
            .timeout(Duration::from_secs(30))
            .header("appVersion", APP_VERSION)
            .header("clientId", &self.client_id)
            .header("clientType", "1")
            .header("iotVersion", "0")
            .header("timestamp", ms_timestamp())
            .header("User-Agent", user_agent());
        if let Some(token) = token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        req
    }

    pub async fn get_iot_key(&self, token: &str) -> anyhow::Result<IotKey> {
        cache_get(
            CacheGetOptions {
                topic: "undoc-api",
                key: "iot-key",
                soft_ttl: HALF_DAY,
                hard_ttl: HALF_DAY,
                negative_ttl: Duration::from_secs(10),
                allow_stale: false,
            },
            async {
                let response = self
                    .app_request(
                        Method::GET,
                        "https://app2.govee.com/app/v1/account/iot/key",
                        Some(token),
                    )
                    .send()
                    .await?;

                #[derive(Deserialize, Debug)]
                #[allow(non_snake_case, dead_code)]
                struct Response {
                    data: IotKey,
                    message: String,
                    status: u64,
                }

                let resp: Response = http_response_body(response).await?;

                Ok(CacheComputeResult::Value(resp.data))
            },
        )
        .await
    }

    pub fn invalidate_account_login(&self) {
        // best-effort: a failed invalidation just means the next call recomputes
        crate::cache::invalidate_key("undoc-api", "account-info").ok();
    }

    async fn login_account_impl(&self) -> anyhow::Result<CacheComputeResult<LoginAccountResponse>> {
        let response = self
            .app_request(
                Method::POST,
                "https://app2.govee.com/account/rest/account/v1/login",
                None,
            )
            .json(&serde_json::json!({
                "email": self.email,
                "password": self.password,
                "client": &self.client_id,
            }))
            .send()
            .await?;

        let resp: Response = http_response_body(response).await?;

        #[derive(Deserialize, Serialize, Debug)]
        #[allow(non_snake_case, dead_code)]
        struct Response {
            client: LoginAccountResponse,
            message: String,
            status: u64,
        }

        let ttl = Duration::from_secs(resp.client.token_expire_cycle as u64);
        Ok(CacheComputeResult::WithTtl(resp.client, ttl))
    }

    pub async fn login_account_cached(&self) -> anyhow::Result<LoginAccountResponse> {
        cache_get(
            CacheGetOptions {
                topic: "undoc-api",
                key: "account-info",
                soft_ttl: HALF_DAY,
                hard_ttl: HALF_DAY,
                negative_ttl: FIFTEEN_MINS,
                allow_stale: false,
            },
            async { self.login_account_impl().await },
        )
        .await
    }

    /// Send an IoT message to a device via Govee's REST relay
    /// (`fx-device/iot-msgs`). The app uses this for SHARED devices instead of
    /// publishing MQTT directly: the message is wrapped in `iotMsg` and
    /// accompanied by the account/device topics and the `gas` token, which
    /// authorizes control of a shared device. Devices ignore direct MQTT
    /// publishes that lack this.
    ///
    /// `inner_msg` is the complete `msg` object (cmd, data, cmdVersion, type);
    /// we add the `transaction` and `accountTopic`.
    pub async fn control_device(
        &self,
        device: &DeviceEntry,
        mut inner_msg: serde_json::Map<String, JsonValue>,
    ) -> anyhow::Result<()> {
        let account = self.login_account_cached().await?;
        let account_topic = account.topic.to_string();
        let transaction = format!("v_{}000", ms_timestamp());

        inner_msg.insert("transaction".into(), json!(transaction));
        inner_msg.insert("accountTopic".into(), json!(account_topic));
        let iot_msg = serde_json::to_string(&json!({ "msg": inner_msg }))?;

        let mut body = serde_json::Map::new();
        body.insert("sku".into(), json!(device.sku));
        body.insert("device".into(), json!(device.device));
        body.insert("gd".into(), json!(device.device_topic()?));
        body.insert("ga".into(), json!(account_topic));
        if let Some(gas) = &device.gas {
            body.insert("gas".into(), json!(gas));
        }
        body.insert("transaction".into(), json!(transaction));
        body.insert("iotMsg".into(), json!(iot_msg));

        let response = self
            .app_request(
                Method::POST,
                "https://app2.govee.com/bff-app/v1/fx-device/iot-msgs",
                Some(account.token.as_str()),
            )
            .json(&body)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_account_login();
        }
        anyhow::ensure!(
            response.status().is_success(),
            "fx-device/iot-msgs failed: {}",
            response.status()
        );
        Ok(())
    }

    pub async fn get_device_list(&self, token: &str) -> anyhow::Result<DevicesResponse> {
        // The app migrated device-list to this BFF GET; the legacy
        // POST /device/rest/devices/v1/list is gone from the app (v7.4.40).
        let response = self
            .app_request(
                Method::GET,
                "https://app2.govee.com/bff-app/v1/device/list",
                Some(token),
            )
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_account_login();
        }

        let envelope: DeviceListEnvelope = http_response_body(response).await?;

        Ok(envelope.data)
    }

    pub fn invalidate_community_login(&self) {
        // best-effort: a failed invalidation just means the next call recomputes
        crate::cache::invalidate_key("undoc-api", "community-login").ok();
    }

    /// Login to community-api.govee.com and return the bearer token
    pub async fn login_community(&self) -> anyhow::Result<String> {
        cache_get(
            CacheGetOptions {
                topic: "undoc-api",
                key: "community-login",
                soft_ttl: ONE_DAY,
                // hard_ttl bounds how long the row survives in sqlite; it must
                // be >= the dynamic soft TTL below (capped at ONE_DAY) or a
                // still-valid token gets evicted early and re-fetched.
                hard_ttl: ONE_WEEK,
                negative_ttl: Duration::from_secs(10),
                allow_stale: false,
            },
            async {
                let response = crate::http_client()
                    .request(Method::POST, "https://community-api.govee.com/os/v1/login")
                    .timeout(Duration::from_secs(60))
                    .json(&serde_json::json!({
                        "email": self.email,
                        "password": self.password,
                    }))
                    .send()
                    .await?;

                #[derive(Deserialize, Debug)]
                #[allow(non_snake_case, dead_code)]
                struct Response {
                    data: ResponseData,
                    message: String,
                    status: u64,
                }

                #[derive(Deserialize, Debug)]
                #[allow(non_snake_case, dead_code)]
                struct ResponseData {
                    email: String,
                    expiredAt: u64,
                    headerUrl: String,
                    id: u64,
                    nickName: String,
                    token: String,
                }

                let resp: Response = http_response_body(response).await?;

                let ttl_ms = resp.data.expiredAt as u128 - epoch_millis();
                let ttl = Duration::from_millis(ttl_ms as u64).min(ONE_DAY);

                Ok(CacheComputeResult::WithTtl(resp.data.token, ttl))
            },
        )
        .await
    }

    pub async fn get_scenes_for_device(sku: &str) -> anyhow::Result<Vec<LightEffectCategory>> {
        let key = format!("scenes-{sku}");

        cache_get(
            CacheGetOptions {
                topic: "undoc-api",
                key: &key,
                soft_ttl: ONE_DAY,
                hard_ttl: ONE_WEEK,
                negative_ttl: Duration::from_secs(1),
                allow_stale: true,
            },
            async {
                let response = crate::http_client()
                    .request(
                        Method::GET,
                        format!(
                            "https://app2.govee.com/appsku/v1/light-effect-libraries?sku={sku}"
                        ),
                    )
                    .timeout(Duration::from_secs(10))
                    .header("AppVersion", APP_VERSION)
                    .header("User-Agent", user_agent())
                    .send()
                    .await?;

                let resp: LightEffectLibraryResponse = http_response_body(response).await?;

                Ok(CacheComputeResult::Value(resp.data.categories))
            },
        )
        .await
    }

    /// This is present primarily to workaround a bug where Govee aren't returning
    /// the full list of scenes via their supported platform API
    pub async fn synthesize_platform_api_scene_list(
        sku: &str,
    ) -> anyhow::Result<Vec<DeviceCapability>> {
        let catalog = Self::get_scenes_for_device(sku).await?;
        let mut options = vec![];

        for c in catalog {
            for s in c.scenes {
                if let Some(param_id) = s.light_effects.first().map(|e| e.scence_param_id) {
                    options.push(EnumOption {
                        name: s.scene_name,
                        value: json!({
                            "paramId": param_id,
                            "id": s.scene_id,
                        }),
                        extras: Default::default(),
                    });
                }
            }
        }

        Ok(vec![DeviceCapability {
            kind: DeviceCapabilityKind::DynamicScene,
            parameters: Some(DeviceParameters::Enum { options }),
            alarm_type: None,
            event_state: None,
            instance: "lightScene".to_string(),
        }])
    }

    pub async fn get_saved_one_click_shortcuts(
        &self,
        community_token: &str,
    ) -> anyhow::Result<Vec<OneClickComponent>> {
        cache_get(
            CacheGetOptions {
                topic: "undoc-api",
                key: "one-click-shortcuts",
                soft_ttl: ONE_DAY,
                hard_ttl: ONE_WEEK,
                negative_ttl: Duration::from_secs(1),
                allow_stale: true,
            },
            async {
                let response = self
                    .app_request(
                        Method::GET,
                        "https://app2.govee.com/bff-app/v1/exec-plat/home",
                        Some(community_token),
                    )
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await?;

                if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                    self.invalidate_community_login();
                }

                let resp: OneClickResponse = http_response_body(response).await?;

                Ok(CacheComputeResult::Value(resp.data.components))
            },
        )
        .await
    }

    pub async fn parse_one_clicks(&self) -> anyhow::Result<Vec<ParsedOneClick>> {
        let token = self.login_community().await?;
        let res = self.get_saved_one_click_shortcuts(&token).await?;
        let mut result = vec![];

        for group in res {
            for oc in group.one_clicks {
                if oc.iot_rules.is_empty() {
                    continue;
                }

                let name = format!("One-Click: {}: {}", group.name, oc.name);

                let mut entries = vec![];
                for rule in oc.iot_rules {
                    if let Some(topic) = rule.device_obj.topic {
                        let msgs = rule.rule.into_iter().map(|r| r.iot_msg).collect();
                        entries.push(ParsedOneClickEntry { topic, msgs });
                    }
                }

                result.push(ParsedOneClick { name, entries });
            }
        }
        Ok(result)
    }
}
