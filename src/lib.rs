#![allow(clippy::missing_safety_doc)]
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::{Arc, LazyLock, Mutex};

mod codec;
mod config;
mod error;
mod ffi;
mod pubsub;

use config::PluginConfig;
use ffi::*;
use pubsub::PubSubKeeper;

// Global state mimicking the Go plugin's `var plugin Keeper`
static KEEPER: LazyLock<Mutex<Option<Arc<PubSubKeeper>>>> = LazyLock::new(|| Mutex::new(None));

#[unsafe(no_mangle)]
pub unsafe extern "C" fn FLBPluginRegister(ctx: *mut c_void) -> c_int {
    let def = ctx as *mut flb_plugin_proxy_def;
    unsafe {
        (*def).ptype = FLB_PROXY_OUTPUT_PLUGIN;
        (*def).proxy = FLB_PROXY_GOLANG;
        (*def).flags = 0;

        let name = CString::new("pubsub").unwrap();
        let desc = CString::new("output pubsub").unwrap();

        // Use libc strdup so Fluent Bit can free it or just leak it since Go plugin leaks it
        (*def).name = libc::strdup(name.as_ptr());
        (*def).description = libc::strdup(desc.as_ptr());
        (*def).event_type = 0;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn FLBPluginInit(ctx: *mut c_void) -> c_int {
    env_logger::try_init().ok();

    let cfg = match PluginConfig::load(ctx) {
        Ok(cfg) => cfg,
        Err(e) => {
            log::error!("[err][init] {}", e);
            return FLB_ERROR;
        }
    };

    log::info!(
        "[pubsub-rs] plugin parameter project = '{}'",
        cfg.project.as_deref().unwrap_or("")
    );
    log::info!(
        "[pubsub-rs] plugin parameter topic = '{}'",
        cfg.topic.as_deref().unwrap_or("")
    );
    log::info!("[pubsub-rs] plugin parameter debug = '{}'", cfg.debug);
    log::info!(
        "[pubsub-rs] PUBSUB_EMULATOR_HOST = '{}'",
        std::env::var("PUBSUB_EMULATOR_HOST").unwrap_or_default()
    );

    let keeper = match PubSubKeeper::new(cfg) {
        Ok(k) => k,
        Err(e) => {
            log::error!("[err][init] {}", e);
            return FLB_ERROR;
        }
    };

    if let Ok(mut g) = KEEPER.lock() {
        *g = Some(Arc::new(keeper));
    }

    FLB_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn FLBPluginFlush(
    data: *const c_void,
    length: c_int,
    tag: *const c_char,
) -> c_int {
    log::info!("[pubsub-rs] flush called length={}", length);
    let keeper_arc = {
        let g = KEEPER.lock().unwrap();
        match &*g {
            Some(k) => k.clone(),
            None => return FLB_ERROR,
        }
    };

    let slice = unsafe { std::slice::from_raw_parts(data as *const u8, length as usize) };

    let tagname = if !tag.is_null() {
        unsafe { CStr::from_ptr(tag) }
            .to_string_lossy()
            .into_owned()
    } else {
        String::new()
    };

    match keeper_arc.flush(slice, &tagname) {
        Ok(_) => FLB_OK,
        Err(pubsub::FlushError::Retryable(e)) => {
            log::error!("[err][publish][retry] {:?}", e);
            FLB_RETRY
        }
        Err(pubsub::FlushError::Fatal(e)) => {
            log::error!("[err][publish][don't retry] {:?}", e);
            FLB_ERROR
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn FLBPluginExit() -> c_int {
    if let Some(keeper) = KEEPER.lock().ok().and_then(|mut g| g.take()) {
        keeper.stop();
    }
    FLB_OK
}
