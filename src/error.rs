use thiserror::Error;

#[derive(Error, Debug)]
pub enum PluginError {
    #[error("Codec error: {0}")]
    Codec(String),
    #[error("Init error: {0}")]
    Init(String),
}
