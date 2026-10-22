use crate::error::PluginError;
use crate::ffi::get_config_key;
use std::os::raw::c_void;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct PluginConfig {
    pub project: String,
    pub topic: String,
    pub jwt: Option<String>,
    pub jwt_path: Option<String>,
    pub message_key: Option<String>,
    pub debug: bool,
    pub timeout: u64,
    pub delay_threshold: Option<Duration>,
    pub byte_threshold: Option<u32>,
    pub count_threshold: Option<u32>,
}

impl PluginConfig {
    pub fn load(ctx: *mut c_void) -> Result<Self, PluginError> {
        Self::from_properties(|key| get_config_key(ctx, key))
    }

    fn from_properties<F>(get: F) -> Result<Self, PluginError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let project = required(&get, "Project")?;
        let topic = required(&get, "Topic")?;
        let jwt = get("Jwt");
        let jwt_path = get("JwtPath");
        if jwt.is_some() && jwt_path.is_some() {
            return Err(PluginError::Init(
                "Jwt and JwtPath cannot be specified together".to_string(),
            ));
        }

        let message_key = get("MessageKey");
        let debug = get("Debug")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let timeout = parse_positive_u64(&get, "Timeout")?.unwrap_or(60_000); // 60s default
        let delay_threshold =
            parse_positive_u64(&get, "DelayThreshold")?.map(Duration::from_millis);
        let byte_threshold = parse_positive_u32(&get, "ByteThreshold")?;
        let count_threshold = parse_positive_u32(&get, "CountThreshold")?;

        Ok(Self {
            project,
            topic,
            jwt,
            jwt_path,
            message_key,
            debug,
            timeout,
            delay_threshold,
            byte_threshold,
            count_threshold,
        })
    }
}

fn required<F>(get: &F, key: &str) -> Result<String, PluginError>
where
    F: Fn(&str) -> Option<String>,
{
    get(key).ok_or_else(|| PluginError::Init(format!("{} is required", key)))
}

fn parse_positive_u64<F>(get: &F, key: &str) -> Result<Option<u64>, PluginError>
where
    F: Fn(&str) -> Option<String>,
{
    get(key)
        .map(|value| {
            value
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| PluginError::Init(format!("{} must be a positive integer", key)))
        })
        .transpose()
}

fn parse_positive_u32<F>(get: &F, key: &str) -> Result<Option<u32>, PluginError>
where
    F: Fn(&str) -> Option<String>,
{
    get(key)
        .map(|value| {
            value
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| PluginError::Init(format!("{} must be a positive integer", key)))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(values: &[(&str, &str)]) -> Result<PluginConfig, PluginError> {
        let values: HashMap<_, _> = values.iter().cloned().collect();
        PluginConfig::from_properties(|key| values.get(key).map(|value| (*value).to_string()))
    }

    #[test]
    fn requires_project_and_topic() {
        assert!(
            config(&[])
                .unwrap_err()
                .to_string()
                .contains("Project is required")
        );
        assert!(
            config(&[("Project", "project")])
                .unwrap_err()
                .to_string()
                .contains("Topic is required")
        );
    }

    #[test]
    fn rejects_conflicting_credential_sources() {
        let error = config(&[
            ("Project", "project"),
            ("Topic", "topic"),
            ("Jwt", "{}"),
            ("JwtPath", "/credentials.json"),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("cannot be specified together"));
    }

    #[test]
    fn parses_batching_and_message_key_options() {
        let config = config(&[
            ("Project", "project"),
            ("Topic", "topic"),
            ("MessageKey", "data"),
            ("DelayThreshold", "25"),
            ("ByteThreshold", "1024"),
            ("CountThreshold", "100"),
        ])
        .unwrap();

        assert_eq!(config.message_key.as_deref(), Some("data"));
        assert_eq!(config.delay_threshold, Some(Duration::from_millis(25)));
        assert_eq!(config.byte_threshold, Some(1024));
        assert_eq!(config.count_threshold, Some(100));
    }

    #[test]
    fn rejects_invalid_positive_integer_options() {
        for (key, value) in [
            ("Timeout", "0"),
            ("DelayThreshold", "-1"),
            ("ByteThreshold", "not-a-number"),
            ("CountThreshold", "0"),
        ] {
            let error =
                config(&[("Project", "project"), ("Topic", "topic"), (key, value)]).unwrap_err();
            assert!(error.to_string().contains(key));
        }
    }
}
