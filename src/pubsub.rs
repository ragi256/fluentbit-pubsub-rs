use crate::codec::{convert_to_json_bytes, decode_records};
use crate::config::PluginConfig;
use crate::error::PluginError;
use google_cloud_auth::credentials::anonymous::Builder as AnonymousCredentials;
use google_cloud_gax::error::rpc::Code;
use google_cloud_pubsub::client::Publisher;
use google_cloud_pubsub::error::PublishError;
use google_cloud_pubsub::model::Message;
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
}

impl PubSubKeeper {
    pub fn new(cfg: PluginConfig) -> Result<Self, PluginError> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| PluginError::Init(format!("tokio rt error: {}", e)))?;

        if let Some(jwt_path) = &cfg.jwt_path {
            unsafe { std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", jwt_path) };
        } else if let Some(jwt) = &cfg.jwt {
            // Write JWT to a temporary file, then point GOOGLE_APPLICATION_CREDENTIALS to it
            // Or use google_cloud_auth's direct JWT loader if supported. For simplicity,
            // setting GOOGLE_APPLICATION_CREDENTIALS_JSON or writing a fast tmp file.
            unsafe { std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS_JSON", jwt) };
        }

        let project = cfg.project.unwrap_or_else(|| "default-project".to_string());
        let topic_name = cfg.topic.unwrap_or_else(|| "default-topic".to_string());
        let full_topic_name = format!("projects/{}/topics/{}", project, topic_name);

        let mut publisher_builder = Publisher::builder(full_topic_name);
        if let Ok(emulator_host) = std::env::var("PUBSUB_EMULATOR_HOST") {
            let emulator_host = emulator_host.trim();
            if !emulator_host.is_empty() {
                let endpoint = if emulator_host.starts_with("http://")
                    || emulator_host.starts_with("https://")
                {
                    emulator_host.to_string()
                } else {
                    format!("http://{}", emulator_host)
                };
                publisher_builder = publisher_builder
                    .with_endpoint(endpoint)
                    .with_credentials(AnonymousCredentials::new().build());
            }
        }

        // Wait for connection
        let publisher = rt
            .block_on(async { publisher_builder.build().await })
            .map_err(|e| PluginError::Init(format!("pubsub publisher init error: {}", e)))?;

        Ok(Self {
            rt,
            publisher,
            timeout_duration: Duration::from_millis(cfg.timeout),
        })
    }

    pub fn flush(&self, data: &[u8], _tag: &str) -> Result<(), FlushError> {
        let records = decode_records(data).map_err(|e| FlushError::Fatal(e.to_string()))?;

        if records.is_empty() {
            return Ok(());
        }

        let mut awaiters = Vec::with_capacity(records.len());

        for (_ts, record_map) in records {
            // Future improvement: Schema registry for actual Avro support.
            // Currently fallback to JSON bytes format for everything.
            let payload =
                convert_to_json_bytes(&record_map).map_err(|e| FlushError::Fatal(e.to_string()))?;

            let msg = Message::new().set_data(payload);
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

#[cfg(test)]
mod tests {
    use super::*;
    use google_cloud_gax::error::rpc::{Code, Status};
    use google_cloud_pubsub::Error;
    use google_cloud_pubsub::error::PublishError;
    use std::sync::Arc;

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
