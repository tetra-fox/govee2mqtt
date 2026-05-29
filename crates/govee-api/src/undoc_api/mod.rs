use crate::cache::{CacheComputeResult, CacheGetOptions, cache_get};
use crate::error::{ApiResult, GoveeApiError};
use crate::http::http_response_body;
use crate::model::{DeviceCapability, DeviceCapabilityKind, DeviceParameters, EnumOption};
use crate::opt_env_var;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

fn network<E: std::fmt::Display>(context: &str) -> impl FnOnce(E) -> GoveeApiError + '_ {
    move |err| GoveeApiError::Network(format!("{context}: {err}").into())
}

fn protocol<E: std::fmt::Display>(context: &str) -> impl FnOnce(E) -> GoveeApiError + '_ {
    move |err| GoveeApiError::Protocol(format!("{context}: {err}"))
}

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
    pub fn opt_email(&self) -> ApiResult<Option<String>> {
        match &self.govee_email {
            Some(key) => Ok(Some(key.to_string())),
            None => opt_env_var("GOVEE2MQTT_EMAIL"),
        }
    }

    pub fn email(&self) -> ApiResult<String> {
        self.opt_email()?.ok_or_else(|| {
            GoveeApiError::Auth(
                "specify the Govee account email either via the \
                 --govee-email parameter or by setting $GOVEE2MQTT_EMAIL"
                    .into(),
            )
        })
    }

    pub fn opt_password(&self) -> ApiResult<Option<String>> {
        match &self.govee_password {
            Some(key) => Ok(Some(key.to_string())),
            None => opt_env_var("GOVEE2MQTT_PASSWORD"),
        }
    }

    pub fn password(&self) -> ApiResult<String> {
        self.opt_password()?.ok_or_else(|| {
            GoveeApiError::Auth(
                "specify the Govee account password either via the \
                 --govee-password parameter or by setting $GOVEE2MQTT_PASSWORD"
                    .into(),
            )
        })
    }

    pub fn api_client(&self) -> ApiResult<GoveeUndocumentedApi> {
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

    pub async fn get_iot_key(&self, token: &str) -> ApiResult<IotKey> {
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
                    .await
                    .map_err(network("requesting iot key"))?;

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

    async fn login_account_impl(&self) -> ApiResult<CacheComputeResult<LoginAccountResponse>> {
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
            .await
            .map_err(network("login_account"))?;

        let resp: Response = http_response_body(response).await?;

        #[derive(Deserialize, Serialize, Debug)]
        #[allow(non_snake_case, dead_code)]
        struct Response {
            client: LoginAccountResponse,
            message: String,
            status: u64,
        }

        // token_expire_cycle is in minutes (see the Govee app's
        // RefreshTokenConfig.getRefreshTokenCheckCycleMinutes), not seconds.
        let ttl = Duration::from_secs(resp.client.token_expire_cycle as u64 * 60);
        Ok(CacheComputeResult::WithTtl(resp.client, ttl))
    }

    pub async fn login_account_cached(&self) -> ApiResult<LoginAccountResponse> {
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
    ) -> ApiResult<()> {
        let account = self.login_account_cached().await?;
        let account_topic = account.topic.to_string();
        let transaction = format!("v_{}000", ms_timestamp());

        inner_msg.insert("transaction".into(), json!(transaction));
        inner_msg.insert("accountTopic".into(), json!(account_topic));
        // The body is JSON we built locally from owned types; serialization
        // cannot fail.
        let iot_msg = serde_json::to_string(&json!({ "msg": inner_msg }))
            .expect("locally-built JSON serializes");

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
            .await
            .map_err(network("fx-device/iot-msgs"))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_account_login();
            return Err(GoveeApiError::Auth(
                "fx-device/iot-msgs rejected the session token".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(GoveeApiError::Api(format!(
                "fx-device/iot-msgs failed: {}",
                response.status()
            )));
        }
        Ok(())
    }

    /// Read a device's stored "common data" blob. The app keeps per-device UI
    /// state (eg: the H6093's full aurora/laser settings) in this cloud store,
    /// keyed by `biz_type` + `biz_key` (the H6093 uses biz_type 3, key
    /// `H6093_<device>`). Returns the inner JSON the app stored, or None if the
    /// device has no record yet. This is how we seed the current device state
    /// before a single-field edit, matching how the app reads it back.
    pub async fn get_common_datas(
        &self,
        biz_type: i32,
        biz_key: &str,
    ) -> ApiResult<Option<JsonValue>> {
        let account = self.login_account_cached().await?;
        let url = reqwest::Url::parse_with_params(
            "https://app2.govee.com/appsku/v1/devices/common-datas",
            &[
                ("bizType", biz_type.to_string()),
                ("bizKey", biz_key.to_string()),
            ],
        )
        .map_err(protocol("building common-datas url"))?;
        let response = self
            .app_request(Method::GET, url, Some(account.token.as_str()))
            .send()
            .await
            .map_err(network("common-datas read"))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_account_login();
            return Err(GoveeApiError::Auth(
                "common-datas read rejected the session token".into(),
            ));
        }
        if !response.status().is_success() {
            return Err(GoveeApiError::Api(format!(
                "common-datas read failed: {}",
                response.status()
            )));
        }

        #[derive(Deserialize)]
        struct Response {
            data: Option<Data>,
        }
        #[derive(Deserialize)]
        struct Data {
            #[serde(rename = "commonDatas")]
            common_datas: Option<String>,
        }

        let resp: Response = http_response_body(response).await?;
        let Some(raw) = resp.data.and_then(|d| d.common_datas) else {
            return Ok(None);
        };
        // The stored value is itself a JSON string.
        Ok(Some(
            serde_json::from_str(&raw).map_err(protocol("parsing inner common-datas JSON"))?,
        ))
    }

    pub async fn get_device_list(&self, token: &str) -> ApiResult<DevicesResponse> {
        // The app migrated device-list to this BFF GET; the legacy
        // POST /device/rest/devices/v1/list is gone from the app (v7.4.40).
        let response = self
            .app_request(
                Method::GET,
                "https://app2.govee.com/bff-app/v1/device/list",
                Some(token),
            )
            .send()
            .await
            .map_err(network("device list"))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.invalidate_account_login();
            return Err(GoveeApiError::Auth(
                "device list rejected the session token".into(),
            ));
        }

        let envelope: DeviceListEnvelope = http_response_body(response).await?;

        Ok(envelope.data)
    }

    pub fn invalidate_community_login(&self) {
        // best-effort: a failed invalidation just means the next call recomputes
        crate::cache::invalidate_key("undoc-api", "community-login").ok();
    }

    /// Login to community-api.govee.com and return the bearer token
    pub async fn login_community(&self) -> ApiResult<String> {
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
                    .await
                    .map_err(network("community login"))?;

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

                let ttl_ms = (resp.data.expiredAt as u128).saturating_sub(epoch_millis());
                let ttl = Duration::from_millis(ttl_ms as u64).min(ONE_DAY);

                Ok(CacheComputeResult::WithTtl(resp.data.token, ttl))
            },
        )
        .await
    }

    pub async fn get_scenes_for_device(sku: &str) -> ApiResult<Vec<LightEffectCategory>> {
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
                    .await
                    .map_err(network("scenes for device"))?;

                let resp: LightEffectLibraryResponse = http_response_body(response).await?;

                Ok(CacheComputeResult::Value(resp.data.categories))
            },
        )
        .await
    }

    /// This is present primarily to workaround a bug where Govee aren't returning
    /// the full list of scenes via their supported platform API
    pub async fn synthesize_platform_api_scene_list(sku: &str) -> ApiResult<Vec<DeviceCapability>> {
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

    /// Capabilities the platform API doesn't report for the H6093 star
    /// projector. They are control-only (no platform-API instance), so the
    /// command path routes them through the IoT ptReal frame encoder keyed by
    /// the same instance name; see `ble::projector::encode_capability`. Returns
    /// empty for any other SKU.
    pub fn synthesize_h6093_capabilities(sku: &str) -> Vec<DeviceCapability> {
        use crate::ble::projector::instance;
        use crate::model::{EnumOption, IntegerRange};
        if sku != "H6093" {
            return vec![];
        }
        let toggle = |inst: &str| DeviceCapability {
            kind: DeviceCapabilityKind::Toggle,
            parameters: None,
            alarm_type: None,
            event_state: None,
            instance: inst.to_string(),
        };
        // A 0-100 slider (relative brightness, speeds, flow).
        let pct = |inst: &str| DeviceCapability {
            kind: DeviceCapabilityKind::Range,
            parameters: Some(DeviceParameters::Integer {
                unit: None,
                range: IntegerRange {
                    min: 0,
                    max: 100,
                    precision: 1,
                },
            }),
            alarm_type: None,
            event_state: None,
            instance: inst.to_string(),
        };
        // The aurora effect picker. Code->name confirmed by capture: tapping the
        // app dropdown Gradient,Breathe,Rainbow,Twinkle produced codes 1,2,4,3 (so
        // 3=Twinkle and 4=Rainbow, NOT the intuitive order).
        let aurora_effect = DeviceCapability {
            kind: DeviceCapabilityKind::Mode,
            parameters: Some(DeviceParameters::Enum {
                options: [
                    (1, "Gradient"),
                    (2, "Breathe"),
                    (4, "Rainbow"),
                    (3, "Twinkle"),
                ]
                .into_iter()
                .map(|(code, name)| EnumOption {
                    name: name.to_string(),
                    value: json!(code),
                    extras: Default::default(),
                })
                .collect(),
            }),
            alarm_type: None,
            event_state: None,
            instance: instance::AURORA_EFFECT.to_string(),
        };
        // The aurora color mode (the app's "Aurora High" toggle): Basic = a single
        // color list, Advanced = separate "waves"/"flows" color sets. Exposed as a
        // select so the control reads as the mode picker it is, rather than an
        // opaque on/off.
        let aurora_color_mode = DeviceCapability {
            kind: DeviceCapabilityKind::Mode,
            parameters: Some(DeviceParameters::Enum {
                options: ["Basic", "Advanced"]
                    .into_iter()
                    .map(|name| EnumOption {
                        name: name.to_string(),
                        value: json!(name),
                        extras: Default::default(),
                    })
                    .collect(),
            }),
            alarm_type: None,
            event_state: None,
            instance: instance::AURORA_COLOR_MODE.to_string(),
        };
        vec![
            // standalone settings toggles
            toggle(instance::PAIRING_STATUS),
            toggle(instance::PAIRING_SOUND),
            toggle(instance::SILENT_POWER_UP),
            toggle(instance::DREAMVIEW_LASER),
            // aurora layer
            toggle(instance::AURORA_ON),
            aurora_color_mode,
            pct(instance::AURORA_BRIGHTNESS),
            pct(instance::AURORA_EFFECT_SPEED),
            pct(instance::AURORA_FLOW),
            aurora_effect,
            // stars (laser) layer
            toggle(instance::STARS_ON),
            pct(instance::STARS_BRIGHTNESS),
            toggle(instance::ORBIT_ON),
            pct(instance::ORBIT_SPEED),
            toggle(instance::FLASHING_ON),
            pct(instance::FLASHING_SPEED),
            // auto-off: enable + "stop playing sound" + a 30-240 minute timeout
            toggle(instance::AUTO_OFF_ENABLE),
            toggle(instance::AUTO_OFF_STOP_SOUND),
            DeviceCapability {
                kind: DeviceCapabilityKind::Range,
                parameters: Some(DeviceParameters::Integer {
                    unit: Some("min".to_string()),
                    range: IntegerRange {
                        min: 30,
                        max: 240,
                        precision: 1,
                    },
                }),
                alarm_type: None,
                event_state: None,
                instance: instance::AUTO_OFF_MINUTES.to_string(),
            },
        ]
    }

    /// Synthesize the H5082 socket's per-outlet countdown entities: a
    /// read-only `*Remaining` sensor and a writable `*Duration` Number for
    /// each (outlet, kind) slot (four slots total). None of these are in the
    /// platform-API capability list, so we add them here and the device's
    /// HTTP info list is augmented at init time to make HA discover them.
    /// Routed back to the wire via [`ble::socket`]. Returns empty for any
    /// other SKU.
    pub fn synthesize_h5082_capabilities(sku: &str) -> Vec<DeviceCapability> {
        use crate::ble::socket::instance;
        use crate::model::IntegerRange;
        if sku != "H5082" {
            return vec![];
        }
        // Remaining-seconds entities are read-only sensors (Property maps to
        // CapabilitySensor in the HA enumerator). Duration entities below are
        // Range so they get editable Number entities.
        let remaining = |inst: &str| DeviceCapability {
            kind: DeviceCapabilityKind::Property,
            parameters: Some(DeviceParameters::Integer {
                unit: Some("s".to_string()),
                range: IntegerRange {
                    min: 0,
                    max: 86_399,
                    precision: 1,
                },
            }),
            alarm_type: None,
            event_state: None,
            instance: inst.to_string(),
        };
        // Duration writes are minutes 0..1439 (24h); 0 is the disarm
        // sentinel. The Number's value also reads back the device's currently
        // armed preset so HA stays in sync if the user sets a countdown from
        // the Govee app.
        let duration = |inst: &str| DeviceCapability {
            kind: DeviceCapabilityKind::Range,
            parameters: Some(DeviceParameters::Integer {
                unit: Some("min".to_string()),
                range: IntegerRange {
                    min: 0,
                    max: 23 * 60 + 59,
                    precision: 1,
                },
            }),
            alarm_type: None,
            event_state: None,
            instance: inst.to_string(),
        };
        vec![
            remaining(instance::O1_AUTO_ON_REMAINING),
            remaining(instance::O1_AUTO_OFF_REMAINING),
            remaining(instance::O2_AUTO_ON_REMAINING),
            remaining(instance::O2_AUTO_OFF_REMAINING),
            duration(instance::O1_AUTO_ON_DURATION),
            duration(instance::O1_AUTO_OFF_DURATION),
            duration(instance::O2_AUTO_ON_DURATION),
            duration(instance::O2_AUTO_OFF_DURATION),
        ]
    }

    pub async fn get_saved_one_click_shortcuts(
        &self,
        community_token: &str,
    ) -> ApiResult<Vec<OneClickComponent>> {
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
                    .await
                    .map_err(network("one-click shortcuts"))?;

                if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                    self.invalidate_community_login();
                    return Err(GoveeApiError::Auth(
                        "one-click shortcuts rejected the community token".into(),
                    ));
                }

                let resp: OneClickResponse = http_response_body(response).await?;

                Ok(CacheComputeResult::Value(resp.data.components))
            },
        )
        .await
    }

    pub async fn parse_one_clicks(&self) -> ApiResult<Vec<ParsedOneClick>> {
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
