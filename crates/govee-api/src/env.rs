use crate::error::{ApiResult, GoveeApiError};
use crate::undoc_api::should_log_sensitive_data;
use std::str::FromStr;

pub fn opt_env_var<T: FromStr>(name: &str) -> ApiResult<Option<T>>
where
    <T as FromStr>::Err: std::fmt::Display,
{
    // Take care: should_log_sensitive_data can recursively call us
    // with name="GOVEE2MQTT_LOG_SENSITIVE_DATA".  We only need to
    // redact values if they are sensitive, and at the time of writing
    // only variables with PASSWORD in their name match this criteria
    let log_sensitive_data = !name.contains("PASSWORD") || should_log_sensitive_data();

    match std::env::var(name) {
        Ok(p) => Ok(Some(p.parse().map_err(|err| {
            let mut message = format!("{err}");
            if !log_sensitive_data {
                message = message.replace(&p, "REDACTED");
            }
            GoveeApiError::Config(format!("parsing ${name}: {message}"))
        })?)),
        Err(std::env::VarError::NotPresent) => {
            let secret_env_name = format!("{}_FILE", name);

            match std::env::var(&secret_env_name) {
                Ok(path) => {
                    let content = std::fs::read_to_string(&path).map_err(|err| {
                        GoveeApiError::Config(format!(
                            "reading secret for {name} from path defined in \
                             {secret_env_name}: {path}: {err}"
                        ))
                    })?;

                    let trimmed_content = content.trim_end();

                    Ok(Some(trimmed_content.parse().map_err(|err| {
                        let mut message = format!("{err}");
                        if !log_sensitive_data {
                            message = message.replace(trimmed_content, "REDACTED");
                        }
                        GoveeApiError::Config(format!(
                            "parsing secret content for {name}: {message}"
                        ))
                    })?))
                }
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(err) => Err(GoveeApiError::Config(format!(
                    "${secret_env_name} is invalid: {err}"
                ))),
            }
        }
        Err(err) => Err(GoveeApiError::Config(format!("${name} is invalid: {err}"))),
    }
}
