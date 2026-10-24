//! The part of `mpv/client.h` muso uses. Declared by hand to avoid a bindgen
//! build step; the layouts match client API 2.x.

use std::ffi::{c_char, c_int, c_void};

#[repr(C)]
pub struct mpv_handle {
    _private: [u8; 0],
}

pub const MPV_FORMAT_FLAG: c_int = 3;
pub const MPV_FORMAT_DOUBLE: c_int = 5;

pub const MPV_EVENT_NONE: c_int = 0;
pub const MPV_EVENT_SHUTDOWN: c_int = 1;
pub const MPV_EVENT_END_FILE: c_int = 7;
pub const MPV_EVENT_FILE_LOADED: c_int = 8;
pub const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;

pub const MPV_END_FILE_REASON_EOF: c_int = 0;
pub const MPV_END_FILE_REASON_ERROR: c_int = 4;

#[repr(C)]
pub struct mpv_event {
    pub event_id: c_int,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_property {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_end_file {
    pub reason: c_int,
    pub error: c_int,
    pub playlist_entry_id: i64,
    pub playlist_insert_id: i64,
    pub playlist_insert_num_entries: c_int,
}

#[link(name = "mpv")]
unsafe extern "C" {
    pub fn mpv_client_api_version() -> std::ffi::c_ulong;
    pub fn mpv_error_string(error: c_int) -> *const c_char;
    pub fn mpv_create() -> *mut mpv_handle;
    pub fn mpv_initialize(ctx: *mut mpv_handle) -> c_int;
    pub fn mpv_terminate_destroy(ctx: *mut mpv_handle);
    pub fn mpv_set_option_string(
        ctx: *mut mpv_handle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int;
    pub fn mpv_set_property_string(
        ctx: *mut mpv_handle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int;
    pub fn mpv_get_property(
        ctx: *mut mpv_handle,
        name: *const c_char,
        format: c_int,
        data: *mut c_void,
    ) -> c_int;
    pub fn mpv_command(ctx: *mut mpv_handle, args: *mut *const c_char) -> c_int;
    pub fn mpv_observe_property(
        ctx: *mut mpv_handle,
        reply_userdata: u64,
        name: *const c_char,
        format: c_int,
    ) -> c_int;
    pub fn mpv_wait_event(ctx: *mut mpv_handle, timeout: f64) -> *mut mpv_event;
    pub fn mpv_set_wakeup_callback(
        ctx: *mut mpv_handle,
        cb: Option<unsafe extern "C" fn(*mut c_void)>,
        d: *mut c_void,
    );
}
