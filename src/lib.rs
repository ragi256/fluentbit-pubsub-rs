#![allow(clippy::missing_safety_doc)]
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};

#[cfg(target_os = "linux")]
mod allocator;
mod codec;
mod config;
mod error;
mod ffi;
mod pubsub;

use config::PluginConfig;
use ffi::*;
use pubsub::PubSubKeeper;

// Fluent Bit can flush the same output concurrently from multiple workers.
const _: fn() = || {
    fn assert_sync<T: Sync>() {}
    assert_sync::<PubSubKeeper>();
};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn FLBPluginRegister(ctx: *mut c_void) -> c_int {
    let def = ctx as *mut flb_plugin_proxy_def;
    unsafe {
        (*def).ptype = FLB_PROXY_OUTPUT_PLUGIN;
        (*def).proxy = FLB_PROXY_GOLANG;
        (*def).flags = 0;

        // Fluent Bit copies the name and borrows the description until it unloads the plugin.
        (*def).name = c"pubsub".as_ptr().cast_mut();
        (*def).description = c"output pubsub".as_ptr().cast_mut();
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn FLBPluginInit(ctx: *mut c_void) -> c_int {
    let plugin = ctx.cast::<flbgo_output_plugin>();
    if plugin.is_null() {
        return FLB_ERROR;
    }
    let proxy_context = unsafe { (*plugin).context };
    if proxy_context.is_null() {
        return FLB_ERROR;
    }

    env_logger::try_init().ok();

    let cfg = match PluginConfig::load(ctx) {
        Ok(cfg) => cfg,
        Err(e) => {
            log::error!("[err][init] {}", e);
            return FLB_ERROR;
        }
    };

    log::info!("[pubsub-rs] plugin parameter project = '{}'", cfg.project);
    log::info!("[pubsub-rs] plugin parameter topic = '{}'", cfg.topic);
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

    // Fluent Bit passes this pointer to the instance's flush and exit callbacks.
    unsafe {
        (*proxy_context).remote_context = Box::into_raw(Box::new(keeper)).cast();
    }

    FLB_OK
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn FLBPluginFlushCtx(
    ctx: *mut c_void,
    data: *const c_void,
    length: usize,
    tag: *const c_char,
) -> c_int {
    log::info!("[pubsub-rs] flush called length={}", length);
    let Some(keeper) = (unsafe { ctx.cast::<PubSubKeeper>().as_ref() }) else {
        return FLB_ERROR;
    };

    let slice = match length {
        0 => &[],
        _ if data.is_null() || length > isize::MAX as usize => return FLB_ERROR,
        _ => unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length) },
    };

    let tagname = if !tag.is_null() {
        unsafe { CStr::from_ptr(tag) }
            .to_string_lossy()
            .into_owned()
    } else {
        String::new()
    };

    match keeper.flush(slice, &tagname) {
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
pub unsafe extern "C" fn FLBPluginExitCtx(ctx: *mut c_void) -> c_int {
    if !ctx.is_null() {
        // Fluent Bit stops the instance's output workers before calling exit.
        let keeper = unsafe { Box::from_raw(ctx.cast::<PubSubKeeper>()) };
        keeper.stop();
    }
    FLB_OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::{null, null_mut};

    #[test]
    fn flush_rejects_null_context() {
        assert_eq!(
            unsafe { FLBPluginFlushCtx(null_mut(), null(), 0, null()) },
            FLB_ERROR
        );
    }

    #[test]
    fn exit_accepts_null_context() {
        assert_eq!(unsafe { FLBPluginExitCtx(null_mut()) }, FLB_OK);
    }
}
