use crate::cache::{cache_get, CacheComputeResult, CacheGetOptions};
use crate::lan_api::{boolean_int, truthy};
use crate::opt_env_var;
use crate::platform_api::{
    from_json, http_response_body, DeviceCapability, DeviceCapabilityKind, DeviceParameters,
    EnumOption,
};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

// <https://github.com/constructorfleet/homebridge-ultimate-govee/blob/main/src/data/clients/RestClient.ts>

const APP_VERSION: &str = "6.5.02";
const HALF_DAY: Duration = Duration::from_secs(3600 * 12);
const ONE_DAY: Duration = Duration::from_secs(86400);
const ONE_WEEK: Duration = Duration::from_secs(86400 * 7);
const FIFTEEN_MINS: Duration = Duration::from_secs(60 * 15);

/// Some data is not meant for human eyes except in very unusual circumstances.
#[derive(Deserialize, Serialize, Clone)]
#[serde(transparent)]
pub struct Redacted<T: std::fmt::Debug>(T);

pub fn should_log_sensitive_data() -> bool {
    if let Ok(Some(v)) = opt_env_var::<String>("GOVEE_LOG_SENSITIVE_DATA") {
        truthy(&v).unwrap_or(false)
    } else {
        false
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Redacted<T> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        if should_log_sensitive_data() {
            self.0.fmt(fmt)
        } else {
            fmt.write_str("REDACTED")
        }
    }
}

impl<T: std::fmt::Debug> std::ops::Deref for Redacted<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

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
    /// the GOVEE_EMAIL environment variable.
    #[arg(long, global = true)]
    pub govee_email: Option<String>,

    /// The password for your Govee account.
    /// If not passed here, it will be read from
    /// the GOVEE_PASSWORD environment variable.
    #[arg(long, global = true)]
    pub govee_password: Option<String>,

    /// Where to store the AWS IoT key file.
    #[arg(long, global = true, default_value = "/dev/shm/govee.iot.key")]
    pub govee_iot_key: PathBuf,

    /// Where to store the AWS IoT certificate file.
    #[arg(long, global = true, default_value = "/dev/shm/govee.iot.cert")]
    pub govee_iot_cert: PathBuf,

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
            None => opt_env_var("GOVEE_EMAIL"),
        }
    }

    pub fn email(&self) -> anyhow::Result<String> {
        self.opt_email()?.ok_or_else(|| {
            anyhow::anyhow!(
                "Please specify the govee account email either via the \
                --govee-email parameter or by setting $GOVEE_EMAIL"
            )
        })
    }

    pub fn opt_password(&self) -> anyhow::Result<Option<String>> {
        match &self.govee_password {
            Some(key) => Ok(Some(key.to_string())),
            None => opt_env_var("GOVEE_PASSWORD"),
        }
    }

    pub fn password(&self) -> anyhow::Result<String> {
        self.opt_password()?.ok_or_else(|| {
            anyhow::anyhow!(
                "Please specify the govee account password either via the \
                --govee-password parameter or by setting $GOVEE_PASSWORD"
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
        let response = self
            .app_request(
                Method::POST,
                "https://app2.govee.com/device/rest/devices/v1/list",
                Some(token),
            )
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_account_login();
        }

        let resp: DevicesResponse = http_response_body(response).await?;

        Ok(resp)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedOneClick {
    pub name: String,
    pub entries: Vec<ParsedOneClickEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedOneClickEntry {
    pub topic: Redacted<String>,
    pub msgs: Vec<JsonValue>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
#[serde(rename_all = "camelCase")]
pub struct IotKey {
    pub endpoint: String,
    pub log: String,
    pub p12: Redacted<String>,
    pub p12_pass: Redacted<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct LightEffectLibraryResponse {
    pub data: LightEffectLibraryCategoryList,
    pub message: String,
    pub status: u32,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct LightEffectLibraryCategoryList {
    pub categories: Vec<LightEffectCategory>,
    pub support_speed: u8,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct LightEffectCategory {
    pub category_id: u32,
    pub category_name: String,
    pub scenes: Vec<LightEffectScene>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct LightEffectScene {
    pub scene_id: u32,
    pub icon_urls: Vec<String>,
    pub scene_name: String,
    pub analytic_name: String,
    pub scene_type: u32,
    pub scene_code: u32,
    pub scence_category_id: u32,
    pub pop_up_prompt: u32,
    pub scenes_hint: String,
    /// Eg: min/max applicable device version constraints
    pub rule: JsonValue,
    pub light_effects: Vec<LightEffectEntry>,
    pub voice_url: String,
    pub create_time: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct LightEffectEntry {
    pub scence_param_id: u32,
    pub scence_name: String,
    /// base64 encoded
    pub scence_param: String,
    pub scene_code: u16,
    pub special_effect: Vec<JsonValue>,
    pub cmd_version: Option<u32>,
    pub scene_type: u32,
    pub diy_effect_code: Vec<JsonValue>,
    pub diy_effect_str: String,
    pub rules: Vec<JsonValue>,
    pub speed_info: JsonValue,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct OneClickResponse {
    pub data: OneClickComponentList,
    pub message: String,
    pub status: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct OneClickComponentList {
    pub components: Vec<OneClickComponent>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct OneClickComponent {
    pub can_disable: Option<u8>,
    #[serde(deserialize_with = "boolean_int")]
    pub can_manage: bool,

    pub feast_type: Option<u64>,
    #[serde(default)]
    pub feasts: Vec<JsonValue>,

    #[serde(default)]
    pub groups: Vec<JsonValue>,

    pub main_device: Option<JsonValue>,

    pub component_id: u64,
    #[serde(default)]
    pub environments: Vec<JsonValue>,
    pub name: String,
    #[serde(rename = "type")]
    pub component_type: u64,

    pub guide_url: Option<String>,
    pub h5_url: Option<String>,
    pub video_url: Option<String>,

    #[serde(default)]
    pub one_clicks: Vec<OneClick>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct OneClick {
    /// Echoes the parent component's id; we don't use it, but newer API
    /// responses include it and debug builds deny unknown fields.
    #[serde(default)]
    pub component_id: Option<u64>,
    pub name: String,
    pub plan_type: i64,
    pub preset_id: i64,
    pub preset_state: i64,
    pub siri_engine_id: i64,
    #[serde(rename = "type")]
    pub rule_type: i64,
    pub desc: String,
    #[serde(default)]
    pub exec_rules: Vec<JsonValue>,
    pub group_id: i64,
    pub group_name: String,
    #[serde(default)]
    pub iot_rules: Vec<OneClickIotRule>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct OneClickIotRule {
    pub device_obj: OneClickIotRuleDevice,
    pub rule: Vec<OneClickIotRuleEntry>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct OneClickIotRuleEntry {
    #[serde(deserialize_with = "embedded_json", serialize_with = "as_json")]
    pub blue_msg: JsonValue,
    pub cmd_type: u64,
    #[serde(deserialize_with = "embedded_json", serialize_with = "as_json")]
    pub cmd_val: OneClickIotRuleEntryCmd,
    pub device_type: u32,
    #[serde(deserialize_with = "embedded_json", serialize_with = "as_json")]
    pub iot_msg: JsonValue,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct OneClickIotRuleEntryCmd {
    pub open: Option<u32>,
    pub scenes_code: Option<u16>,
    pub scence_id: Option<u16>,
    pub scenes_str: Option<String>,
    pub scence_param_id: Option<u16>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct OneClickIotRuleDevice {
    pub name: Option<String>,
    pub device: Option<String>,
    pub sku: Option<String>,

    pub topic: Option<Redacted<String>>,

    pub ble_address: Option<String>,
    pub ble_name: Option<String>,
    pub device_splicing_status: u32,
    pub feast_id: u64,
    pub feast_name: String,
    pub feast_type: u64,
    pub goods_type: Option<u64>,
    pub ic: Option<u32>,
    #[serde(rename = "ic_sub_1")]
    pub ic_sub_1: Option<u32>,
    #[serde(rename = "ic_sub_2")]
    pub ic_sub_2: Option<u32>,
    #[serde(deserialize_with = "boolean_int")]
    pub is_feast: bool,
    pub pact_type: Option<u32>,
    pub pact_code: Option<u32>,

    pub settings: Option<JsonValue>,
    pub spec: Option<String>,
    pub sub_device: String,
    pub sub_device_num: u64,
    pub sub_devices: Option<JsonValue>,

    pub version_hard: Option<String>,
    pub version_soft: Option<String>,
    pub wifi_soft_version: Option<String>,
    pub wifi_hard_version: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LoginAccountResponse {
    #[serde(rename = "A")]
    pub a: Redacted<String>,
    #[serde(rename = "B")]
    pub b: Redacted<String>,
    pub account_id: Redacted<u64>,
    /// this is the client id that we passed in
    pub client: Redacted<String>,
    pub is_savvy_user: bool,
    pub refresh_token: Option<Redacted<String>>,
    pub client_name: Option<String>,
    pub push_token: Option<Redacted<String>>,
    pub version_code: Option<String>,
    pub version_name: Option<String>,
    pub sys_version: Option<String>,
    pub token: Redacted<String>,
    pub token_expire_cycle: u32,
    pub topic: Redacted<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DevicesResponse {
    pub devices: Vec<DeviceEntry>,
    pub groups: Vec<GroupEntry>,
    pub message: String,
    pub status: u16,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GroupEntry {
    pub group_id: u64,
    pub group_name: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct DeviceEntry {
    pub attributes_id: u32,
    pub device_id: Option<u32>,
    pub device: String,
    pub device_ext: DeviceEntryExt,
    pub device_name: String,
    pub goods_type: u32,
    pub group_id: u64,
    pub pact_code: Option<u32>,
    pub pact_type: Option<u32>,
    pub share: Option<u32>,
    pub sku: String,
    pub spec: String,
    #[serde(deserialize_with = "boolean_int")]
    pub support_scene: bool,
    pub version_hard: String,
    pub version_soft: String,
    pub gid_confirmed: Option<bool>,
    /// An opaque access token present on devices that are shared with this
    /// account. We don't use it, but must accept it so debug builds (which
    /// deny unknown fields) don't reject the device list.
    /// <https://github.com/wez/govee2mqtt/issues/76>
    #[serde(default)]
    pub gas: Option<String>,
}

impl DeviceEntry {
    /// Whether this device is shared with the account rather than owned. Shared
    /// devices must be controlled via the REST relay (carrying `gas`), not by
    /// publishing MQTT directly. <https://github.com/wez/govee2mqtt/issues/76>
    pub fn is_shared(&self) -> bool {
        self.share == Some(1)
    }

    pub fn device_topic(&self) -> anyhow::Result<&str> {
        self.device_ext
            .device_settings
            .topic
            .as_ref()
            .map(|t| t.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "device {id} has no topic, is it a BLE-only device?",
                    id = self.device
                )
            })
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct DeviceEntryExt {
    #[serde(deserialize_with = "embedded_json", serialize_with = "as_json")]
    pub device_settings: DeviceSettings,
    #[serde(deserialize_with = "embedded_json", serialize_with = "as_json")]
    pub ext_resources: ExtResources,
    #[serde(deserialize_with = "embedded_json", serialize_with = "as_json")]
    pub last_device_data: LastDeviceData,
    /// Present for devices shared with this account (eg: `{"isFeast":0,
    /// "pushSwitch":1}`). We don't use it, but must accept it so that debug
    /// builds (which deny unknown fields) don't reject the device list.
    /// <https://github.com/wez/govee2mqtt/issues/76>
    #[serde(default)]
    pub shared_settings: Option<JsonValue>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct DeviceSettings {
    /// Maybe be absent for BLE devices
    pub wifi_name: Option<String>,
    pub address: Option<String>,
    pub ble_name: Option<String>,
    pub topic: Option<Redacted<String>>,
    pub wifi_mac: Option<String>,
    pub pact_type: Option<u32>,
    pub pact_code: Option<u32>,
    pub dsp_version_soft: Option<JsonValue>,
    pub wifi_soft_version: Option<String>,
    pub wifi_hard_version: Option<String>,
    pub ic: Option<u32>,
    #[serde(rename = "ic_sub_1")]
    pub ic_sub_1: Option<u32>,
    #[serde(rename = "ic_sub_2")]
    pub ic_sub_2: Option<u32>,
    pub secret_code: Option<Redacted<String>>,
    #[serde(deserialize_with = "boolean_int", default)]
    pub boil_water_completed_noti_on_off: bool,
    #[serde(deserialize_with = "boolean_int", default)]
    pub boil_water_exception_noti_on_off: bool,
    #[serde(deserialize_with = "boolean_int", default)]
    pub completion_noti_on_off: bool,
    #[serde(deserialize_with = "boolean_int", default)]
    pub auto_shut_down_on_off: bool,
    /// Whether the device supports encrypted communication. Present on newer
    /// devices; reported as a JSON bool rather than the 0/1 used elsewhere.
    pub support_enc: Option<bool>,
    #[serde(deserialize_with = "boolean_int", default)]
    pub water_shortage_on_off: bool,
    #[serde(deserialize_with = "boolean_int", default)]
    pub air_quality_on_off: bool,
    pub mcu_soft_version: Option<String>,
    pub mcu_hard_version: Option<String>,
    pub sku: Option<String>,
    pub device: Option<String>,
    pub device_name: Option<String>,
    pub version_hard: Option<String>,
    pub version_soft: Option<String>,
    pub play_state: Option<bool>,
    pub tem_min: Option<i64>,
    pub tem_max: Option<i64>,
    pub tem_warning: Option<bool>,
    pub fah_open: Option<bool>,
    pub tem_cali: Option<i64>,
    pub hum_min: Option<i64>,
    pub hum_max: Option<i64>,
    pub hum_warning: Option<bool>,
    pub hum_cali: Option<i64>,
    pub net_waring: Option<bool>,
    pub upload_rate: Option<i64>,
    pub battery: Option<i64>,
    /// millisecond timestamp
    pub time: Option<u64>,
    pub wifi_level: Option<i64>,

    pub pm25_min: Option<i64>,
    pub pm25_max: Option<i64>,
    pub pm25_warning: Option<bool>,

    /// `{"sub_0": {"name": "Device Name"}}`
    pub sub_devices: Option<JsonValue>,
    pub bd_type: Option<i64>,
    #[serde(deserialize_with = "boolean_int", default)]
    pub filter_expire_on_off: bool,

    /// eg: Glide Hexa. Value is base64 encoded data
    pub shapes: Option<String>,
    pub support_ble_broad_v3: Option<bool>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct ExtResources {
    pub sku_url: Option<String>,
    pub head_on_img_new: Option<String>,
    pub head_on_img: Option<String>,
    pub head_off_img: Option<String>,
    pub head_off_img_new: Option<String>,
    pub ext: Option<String>,
    pub ic: Option<u32>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(debug_assertions, serde(deny_unknown_fields))]
pub struct LastDeviceData {
    pub online: Option<bool>,
    pub bind: Option<bool>,

    pub tem: Option<i64>,
    pub hum: Option<i64>,
    /// timestamp in milliseconds
    pub last_time: Option<u64>,
    pub avg_day_tem: Option<i64>,
    pub avg_day_hum: Option<i64>,
}

pub fn as_json<S, T>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: Serialize,
    S: serde::Serializer,
{
    use serde::ser::Error as _;

    let s = serde_json::to_string(value).map_err(|e| S::Error::custom(format!("{e:#}")))?;

    s.serialize(serializer)
}

pub fn embedded_json<'de, T: DeserializeOwned, D: serde::de::Deserializer<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    use serde::de::Error as _;
    let s = String::deserialize(deserializer)?;
    from_json(if s.is_empty() { "null" } else { &s }).map_err(|e| {
        D::Error::custom(format!(
            "{} {e:#} while processing embedded json text {s}",
            std::any::type_name::<T>()
        ))
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::platform_api::from_json;

    #[test]
    fn get_device_scenes() {
        let resp: DevicesResponse =
            from_json(include_str!("../test-data/undoc-device-list.json")).expect("parse");
        assert!(!resp.devices.is_empty());
    }

    #[test]
    fn get_one_click() {
        let resp: OneClickResponse =
            from_json(include_str!("../test-data/undoc-one-click.json")).expect("parse");
        assert!(!resp.data.components.is_empty());
    }

    #[test]
    fn issue36() {
        let _: OneClickResponse =
            from_json(include_str!("../test-data/undoc-one-click-issue36.json")).expect("parse");
    }

    #[test]
    fn light_effect_library() {
        let _: LightEffectLibraryResponse =
            from_json(include_str!("../test-data/light-effect-library-h6072.json")).expect("parse");
    }

    #[test]
    fn issue_14() {
        let resp: DevicesResponse =
            from_json(include_str!("../test-data/issue14.json")).expect("parse");
        assert!(!resp.devices.is_empty());
    }

    #[test]
    fn issue_21() {
        let resp: DevicesResponse =
            from_json(include_str!("../test-data/undoc-device-list-issue-21.json")).expect("parse");
        assert!(!resp.devices.is_empty());
    }
}
