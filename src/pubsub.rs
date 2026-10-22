use crate::codec::{convert_to_json_bytes, decode_records, record_value_to_payload};
use crate::config::PluginConfig;
use crate::error::PluginError;
use google_cloud_auth::credentials::Credentials;
use google_cloud_auth::credentials::anonymous::Builder as AnonymousCredentials;
use google_cloud_auth::credentials::external_account::Builder as ExternalAccountCredentials;
use google_cloud_auth::credentials::service_account::Builder as ServiceAccountCredentials;
use google_cloud_gax::error::rpc::Code;
use google_cloud_pubsub::client::Publisher;
use google_cloud_pubsub::error::PublishError;
use google_cloud_pubsub::model::Message;
use std::collections::HashMap;
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::time::timeout;

#[derive(Debug)]
pub enum FlushError {
    Retryable(String),
    Fatal(String),
}

/// Classify a PublishError into FlushError based on gRPC status code.
///
/// Retryable codes (transient server/network issues):
///   Unavailable, DeadlineExceeded, Aborted, Internal, ResourceExhausted, Unknown
///
/// Fatal codes (user intervention required):
///   NotFound, PermissionDenied, InvalidArgument, Unauthenticated, etc.
fn classify_publish_error(err: PublishError) -> FlushError {
    match &err {
        PublishError::Rpc(inner) => {
            if let Some(status) = inner.status() {
                let retryable = matches!(
                    status.code,
                    Code::Unavailable
                        | Code::DeadlineExceeded
                        | Code::Aborted
                        | Code::Internal
                        | Code::ResourceExhausted
                        | Code::Unknown
                );
                if retryable {
                    FlushError::Retryable(format!("publish rpc error ({}): {}", status.code, err))
                } else {
                    FlushError::Fatal(format!("publish rpc error ({}): {}", status.code, err))
                }
            } else {
                // No gRPC status available — treat as retryable to be safe
                FlushError::Retryable(format!("publish error (no status): {}", err))
            }
        }
        PublishError::OrderingKeyPaused => FlushError::Fatal(format!("publish error: {}", err)),
        PublishError::Shutdown => FlushError::Fatal(format!("publish error: {}", err)),
        // PublishError is #[non_exhaustive]
        _ => FlushError::Retryable(format!("publish error (unknown variant): {}", err)),
    }
}

pub struct PubSubKeeper {
    rt: Runtime,
    publisher: Publisher,
    timeout_duration: Duration,
    message_key: Option<String>,
    attributes: HashMap<String, String>,
}

impl PubSubKeeper {
    pub fn new(cfg: PluginConfig) -> Result<Self, PluginError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| PluginError::Init(format!("tokio rt error: {}", e)))?;

        let full_topic_name = format!("projects/{}/topics/{}", cfg.project, cfg.topic);

        let mut publisher_builder = Publisher::builder(full_topic_name);
        let emulator_host = std::env::var("PUBSUB_EMULATOR_HOST")
            .ok()
            .filter(|host| !host.trim().is_empty());
        if let Some(endpoint) = cfg.publisher_endpoint(emulator_host.as_deref())? {
            publisher_builder = publisher_builder.with_endpoint(endpoint);
        }
        if emulator_host.is_some() {
            publisher_builder =
                publisher_builder.with_credentials(AnonymousCredentials::new().build());
        } else {
            // The external-account credential builder initializes a Tokio-based token
            // cache. Enter the plugin runtime while building it, without mutating
            // process-wide environment variables.
            let credentials = {
                let _enter = rt.enter();
                credentials_from_config(&cfg)?
            };
            if let Some(credentials) = credentials {
                publisher_builder = publisher_builder.with_credentials(credentials);
            }
        }

        if let Some(delay_threshold) = cfg.delay_threshold {
            publisher_builder = publisher_builder.set_delay_threshold(delay_threshold);
        }
        if let Some(byte_threshold) = cfg.byte_threshold {
            publisher_builder = publisher_builder.set_byte_threshold(byte_threshold);
        }
        if let Some(count_threshold) = cfg.count_threshold {
            publisher_builder = publisher_builder.set_message_count_threshold(count_threshold);
        }

        // Wait for connection
        let publisher = rt
            .block_on(async { publisher_builder.build().await })
            .map_err(|e| PluginError::Init(format!("pubsub publisher init error: {}", e)))?;

        Ok(Self {
            rt,
            publisher,
            timeout_duration: Duration::from_millis(cfg.timeout),
            message_key: cfg.message_key,
            attributes: cfg.attributes,
        })
    }

    pub fn flush(&self, data: &[u8], _tag: &str) -> Result<(), FlushError> {
        let records = decode_records(data).map_err(|e| FlushError::Fatal(e.to_string()))?;

        if records.is_empty() {
            return Ok(());
        }

        let mut awaiters = Vec::with_capacity(records.len());

        for (_ts, record_map) in records {
            let payload = match &self.message_key {
                Some(key) => record_value_to_payload(&record_map, key),
                None => convert_to_json_bytes(&record_map),
            }
            .map_err(|e| FlushError::Fatal(e.to_string()))?;

            let msg = Message::new()
                .set_data(payload)
                .set_attributes(self.attributes.clone());
            awaiters.push(self.publisher.publish(msg));
        }

        let res = self.rt.block_on(async {
            timeout(self.timeout_duration, async {
                for awaiter in awaiters {
                    if let Err(e) = awaiter.await {
                        return Err(classify_publish_error(e));
                    }
                }
                Ok(())
            })
            .await
        });

        match res {
            Ok(inner_res) => inner_res,
            Err(e) => Err(FlushError::Retryable(format!("timeout: {}", e))),
        }
    }

    pub fn stop(&self) {
        self.rt.block_on(async {
            self.publisher.flush().await;
        });
    }
}

fn credentials_from_config(cfg: &PluginConfig) -> Result<Option<Credentials>, PluginError> {
    let contents = match (&cfg.jwt, &cfg.jwt_path) {
        (Some(jwt), None) => jwt.clone(),
        (None, Some(path)) => std::fs::read_to_string(path)
            .map_err(|e| PluginError::Init(format!("failed to read JwtPath '{}': {}", path, e)))?,
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => unreachable!("PluginConfig rejects conflicting credentials"),
    };

    let credentials_json: serde_json::Value = serde_json::from_str(&contents)
        .map_err(|e| PluginError::Init(format!("credential JSON is invalid: {}", e)))?;
    let credential_type = credentials_json
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            PluginError::Init("credential JSON has no string 'type' field".to_string())
        })?;

    let credentials = match credential_type {
        "service_account" => ServiceAccountCredentials::new(credentials_json).build(),
        "external_account" => ExternalAccountCredentials::new(credentials_json).build(),
        unsupported => {
            return Err(PluginError::Init(format!(
                "unsupported credential type '{}'; use service_account or external_account",
                unsupported
            )));
        }
    }
    .map_err(|e| PluginError::Init(format!("failed to configure credentials: {}", e)))?;

    Ok(Some(credentials))
}

#[cfg(test)]
mod tests {
    use super::*;
    use google_cloud_gax::error::rpc::{Code, Status};
    use google_cloud_pubsub::Error;
    use google_cloud_pubsub::error::PublishError;
    use std::sync::Arc;

    fn config_with_jwt(jwt: &str) -> PluginConfig {
        PluginConfig {
            project: "project".to_string(),
            topic: "topic".to_string(),
            jwt: Some(jwt.to_string()),
            jwt_path: None,
            message_key: None,
            attributes: HashMap::new(),
            endpoint: None,
            debug: false,
            timeout: 1_000,
            delay_threshold: None,
            byte_threshold: None,
            count_threshold: None,
        }
    }

    #[test]
    fn rejects_invalid_raw_credential_json() {
        let error = credentials_from_config(&config_with_jwt("not-json")).unwrap_err();
        assert!(error.to_string().contains("credential JSON is invalid"));
    }

    #[test]
    fn accepts_wif_external_account_json_without_environment_mutation() {
        let runtime = Runtime::new().unwrap();
        let _enter = runtime.enter();
        let credentials = credentials_from_config(&config_with_jwt(
            r#"{
                "type": "external_account",
                "audience": "//iam.googleapis.com/projects/123/locations/global/workloadIdentityPools/pool/providers/aws",
                "subject_token_type": "urn:ietf:params:oauth:token-type:jwt",
                "token_url": "https://sts.googleapis.com/v1/token",
                "credential_source": {"file": "/var/run/secrets/token"}
            }"#,
        ));
        assert!(credentials.unwrap().is_some());
    }

    #[test]
    fn test_classify_rpc_retryable() {
        let codes = vec![
            Code::Unavailable,
            Code::DeadlineExceeded,
            Code::Aborted,
            Code::Internal,
            Code::ResourceExhausted,
            Code::Unknown,
        ];

        for code in codes {
            let status = Status::default().set_code(code).set_message("retryable");
            let err = PublishError::Rpc(Arc::new(Error::service(status)));
            let classified = classify_publish_error(err);
            match classified {
                FlushError::Retryable(msg) => assert!(msg.contains(&format!("{}", code))),
                _ => panic!("Expected Retryable for code {:?}", code),
            }
        }
    }

    #[test]
    fn test_classify_rpc_fatal() {
        let codes = vec![
            Code::NotFound,
            Code::PermissionDenied,
            Code::InvalidArgument,
            Code::Unauthenticated,
            Code::AlreadyExists,
            Code::FailedPrecondition,
            Code::OutOfRange,
            Code::Unimplemented,
            Code::DataLoss,
        ];

        for code in codes {
            let status = Status::default().set_code(code).set_message("fatal");
            let err = PublishError::Rpc(Arc::new(Error::service(status)));
            let classified = classify_publish_error(err);
            match classified {
                FlushError::Fatal(msg) => assert!(msg.contains(&format!("{}", code))),
                _ => panic!("Expected Fatal for code {:?}", code),
            }
        }
    }

    #[test]
    fn test_classify_non_rpc_errors() {
        // OrderingKeyPaused -> Fatal
        match classify_publish_error(PublishError::OrderingKeyPaused) {
            FlushError::Fatal(msg) => assert!(msg.contains("paused")),
            _ => panic!("Expected Fatal for OrderingKeyPaused"),
        }

        // Shutdown -> Fatal
        match classify_publish_error(PublishError::Shutdown) {
            FlushError::Fatal(msg) => assert!(msg.contains("shut down")),
            _ => panic!("Expected Fatal for Shutdown"),
        }
    }
}
