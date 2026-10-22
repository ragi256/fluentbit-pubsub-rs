use crate::error::PluginError;
use crate::ffi::get_config_key;
use std::collections::HashMap;
use std::os::raw::c_void;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct PluginConfig {
    pub project: String,
    pub topic: String,
    pub jwt: Option<String>,
    pub jwt_path: Option<String>,
    pub message_key: Option<String>,
    pub attributes: HashMap<String, String>,
    pub endpoint: Option<String>,
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
        let attributes = get("Attributes")
            .map(|value| parse_attributes(&value))
            .transpose()?
            .unwrap_or_default();
        let endpoint = get("Endpoint")
            .map(|value| validate_endpoint(&value))
            .transpose()?;
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
            attributes,
            endpoint,
            debug,
            timeout,
            delay_threshold,
            byte_threshold,
            count_threshold,
        })
    }

    /// An explicit endpoint overrides emulator routing. The emulator variable
    /// still determines whether authentication is anonymous.
    pub fn publisher_endpoint(
        &self,
        emulator_host: Option<&str>,
    ) -> Result<Option<String>, PluginError> {
        let emulator_host = emulator_host.map(str::trim).filter(|host| !host.is_empty());
        if let Some(endpoint) = &self.endpoint {
            if endpoint.starts_with("http://") && emulator_host.is_none() {
                return Err(PluginError::Init(
                    "Endpoint must use HTTPS unless PUBSUB_EMULATOR_HOST is set".to_string(),
                ));
            }
            return Ok(Some(endpoint.clone()));
        }

        emulator_host
            .map(|host| {
                if host.starts_with("http://") || host.starts_with("https://") {
                    validate_endpoint(host)
                } else {
                    validate_endpoint(&format!("http://{}", host))
                }
            })
            .transpose()
    }
}

fn parse_attributes(value: &str) -> Result<HashMap<String, String>, PluginError> {
    let attributes: HashMap<String, String> = serde_json::from_str(value).map_err(|_| {
        PluginError::Init("Attributes must be a JSON object with string values".to_string())
    })?;
    if attributes.len() > 100 {
        return Err(PluginError::Init(
            "Attributes supports at most 100 entries".to_string(),
        ));
    }
    for (key, value) in &attributes {
        if key.is_empty() || key.len() > 256 || key.starts_with("goog") {
            return Err(PluginError::Init(
                "Attributes keys must be non-empty, at most 256 bytes, and not start with 'goog'"
                    .to_string(),
            ));
        }
        if value.len() > 1024 {
            return Err(PluginError::Init(
                "Attributes values must be at most 1024 bytes".to_string(),
            ));
        }
    }
    Ok(attributes)
}

fn validate_endpoint(value: &str) -> Result<String, PluginError> {
    let error = || {
        PluginError::Init(
            concat!(
                "Endpoint must be an HTTP(S) URL with a host and optional port, ",
                "without credentials, path, query, or fragment"
            )
            .to_string(),
        )
    };
    let endpoint = url::Url::parse(value).map_err(|_| error())?;
    if !matches!(endpoint.scheme(), "https" | "http")
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.path() != "/"
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(error());
    }
    Ok(endpoint.as_str().trim_end_matches('/').to_string())
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

    #[test]
    fn optional_publish_settings_preserve_defaults() {
        let config = config(&[("Project", "project"), ("Topic", "topic")]).unwrap();
        assert!(config.attributes.is_empty());
        assert_eq!(config.publisher_endpoint(None).unwrap(), None);
        assert_eq!(config.delay_threshold, None);
        assert_eq!(config.byte_threshold, None);
        assert_eq!(config.count_threshold, None);
    }

    #[test]
    fn attributes_are_strings_and_preserve_punctuation() {
        let config = config(&[
            ("Project", "project"),
            ("Topic", "topic"),
            (
                "Attributes",
                r#"{"service":"api","note":"comma,equals=ok","empty":""}"#,
            ),
        ])
        .unwrap();
        assert_eq!(config.attributes["service"], "api");
        assert_eq!(config.attributes["note"], "comma,equals=ok");
        assert_eq!(config.attributes["empty"], "");

        for invalid in [
            "service=api",
            "[]",
            "null",
            r#"{"count":1}"#,
            r#"{"nested":{}}"#,
        ] {
            assert!(parse_attributes(invalid).is_err(), "accepted {invalid}");
        }
    }

    #[test]
    fn validates_attribute_count_and_utf8_byte_limits() {
        let mut attributes: HashMap<String, String> = (0..100)
            .map(|i| (format!("key{i}"), "value".to_string()))
            .collect();
        assert!(parse_attributes(&serde_json::to_string(&attributes).unwrap()).is_ok());
        attributes.insert("extra".to_string(), "value".to_string());
        assert!(parse_attributes(&serde_json::to_string(&attributes).unwrap()).is_err());

        for (key, value, accepted) in [
            ("a".repeat(256), "v".repeat(1024), true),
            ("a".repeat(257), "value".to_string(), false),
            ("key".to_string(), "v".repeat(1025), false),
            ("あ".repeat(85), "あ".repeat(341), true),
            ("あ".repeat(86), "value".to_string(), false),
            ("key".to_string(), "あ".repeat(342), false),
            ("".to_string(), "value".to_string(), false),
            ("goog-reserved".to_string(), "value".to_string(), false),
        ] {
            let json = serde_json::to_string(&HashMap::from([(key, value)])).unwrap();
            assert_eq!(parse_attributes(&json).is_ok(), accepted);
        }
    }

    #[test]
    fn endpoint_and_emulator_routing_are_explicit() {
        let defaults = config(&[("Project", "project"), ("Topic", "topic")]).unwrap();
        assert_eq!(
            defaults
                .publisher_endpoint(Some(" localhost:8681 "))
                .unwrap()
                .as_deref(),
            Some("http://localhost:8681")
        );
        let custom = config(&[
            ("Project", "project"),
            ("Topic", "topic"),
            ("Endpoint", "https://us-east1-pubsub.googleapis.com/"),
        ])
        .unwrap();
        for emulator in [None, Some("localhost:8681")] {
            assert_eq!(
                custom.publisher_endpoint(emulator).unwrap().as_deref(),
                Some("https://us-east1-pubsub.googleapis.com")
            );
        }

        let local = config(&[
            ("Project", "project"),
            ("Topic", "topic"),
            ("Endpoint", "http://localhost:8681"),
        ])
        .unwrap();
        assert!(local.publisher_endpoint(None).is_err());
        assert!(local.publisher_endpoint(Some(" ")).is_err());
        assert_eq!(
            local
                .publisher_endpoint(Some("unused:8681"))
                .unwrap()
                .as_deref(),
            Some("http://localhost:8681")
        );
    }

    #[test]
    fn rejects_ambiguous_endpoint_urls() {
        for endpoint in [
            "localhost:8681",
            "ftp://localhost",
            "https://",
            "https://host/path",
            "https://user:password@host",
            "https://host?query=value",
            "https://host#fragment",
        ] {
            assert!(validate_endpoint(endpoint).is_err(), "accepted {endpoint}");
        }
    }
}
