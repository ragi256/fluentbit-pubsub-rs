use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};

pub const FLB_ERROR: c_int = 0;
pub const FLB_OK: c_int = 1;
pub const FLB_RETRY: c_int = 2;

pub const FLB_PROXY_OUTPUT_PLUGIN: c_int = 2;
pub const FLB_PROXY_GOLANG: c_int = 11;

#[repr(C)]
pub struct flb_plugin_proxy_def {
    pub ptype: c_int,
    pub proxy: c_int,
    pub flags: c_int,
    pub name: *mut c_char,
    pub description: *mut c_char,
    pub event_type: c_int,
}

#[repr(C)]
struct flb_api {
    output_get_property: Option<unsafe extern "C" fn(*mut c_char, *mut c_void) -> *mut c_char>,
    _reserved: *mut c_char,
}

#[repr(C)]
struct flbgo_output_plugin {
    _reserved: *mut c_void,
    api: *mut flb_api,
    o_ins: *mut c_void,
    _context: *mut c_void,
}

#[cfg(not(test))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn output_get_property(
    key: *const c_char,
    ctx: *mut c_void,
) -> *const c_char {
    if ctx.is_null() || key.is_null() {
        return std::ptr::null();
    }

    let plugin = ctx as *mut flbgo_output_plugin;
    if unsafe { (*plugin).api.is_null() } {
        return std::ptr::null();
    }

    let api = unsafe { &*((*plugin).api) };
    let getter = match api.output_get_property {
        Some(f) => f,
        None => return std::ptr::null(),
    };

    unsafe { getter(key as *mut c_char, (*plugin).o_ins) as *const c_char }
}

#[cfg(test)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn output_get_property(
    _key: *const c_char,
    _ctx: *mut c_void,
) -> *const c_char {
    std::ptr::null()
}

pub fn get_config_key(ctx: *mut c_void, key: &str) -> Option<String> {
    let c_key = CString::new(key).unwrap();
    let val_ptr = unsafe { output_get_property(c_key.as_ptr(), ctx) };
    if val_ptr.is_null() {
        return None;
    }
    let val = unsafe { CStr::from_ptr(val_ptr) }
        .to_string_lossy()
        .into_owned();
    if val.is_empty() { None } else { Some(val) }
}
