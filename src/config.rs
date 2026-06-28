use crate::error::PluginError;
use crate::ffi::get_config_key;
use std::os::raw::c_void;

#[derive(Debug, Clone)]
pub struct PluginConfig {
    pub project: Option<String>,
    pub topic: Option<String>,
    pub jwt: Option<String>,
    pub jwt_path: Option<String>,
    pub debug: bool,
    pub timeout: u64,
}

impl PluginConfig {
    pub fn load(ctx: *mut c_void) -> Result<Self, PluginError> {
        let project = get_config_key(ctx, "Project");
        let topic = get_config_key(ctx, "Topic");
        let jwt = get_config_key(ctx, "Jwt");
        let jwt_path = get_config_key(ctx, "JwtPath");
        let debug = get_config_key(ctx, "Debug")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let timeout = get_config_key(ctx, "Timeout")
            .and_then(|v| v.parse().ok())
            .unwrap_or(60000); // 60s default

        Ok(Self {
            project,
            topic,
            jwt,
            jwt_path,
            debug,
            timeout,
        })
    }
}
