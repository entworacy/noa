//! Frida Core ABI. Only the process supervisor uses this module.
use std::{
    ffi::{CStr, c_char, c_int, c_uint, c_void},
    ptr,
};
#[repr(C)]
pub(super) struct GError {
    domain: c_uint,
    code: c_int,
    message: *mut c_char,
}

pub(super) enum FridaDeviceManager {}
pub(super) enum FridaDevice {}
pub(super) enum GMainContext {}
pub(super) enum GCancellable {}
pub(super) enum GBytes {}

unsafe extern "C" {
    pub(super) fn frida_init();
    pub(super) fn frida_selinux_patch_policy();
    pub(super) fn frida_deinit();
    pub(super) fn frida_version_string() -> *const c_char;
    pub(super) fn frida_unref(value: *mut c_void);
    pub(super) fn frida_get_main_context() -> *mut GMainContext;
    pub(super) fn frida_device_manager_new() -> *mut FridaDeviceManager;
    pub(super) fn frida_device_manager_close_sync(
        manager: *mut FridaDeviceManager,
        cancellable: *mut GCancellable,
        error: *mut *mut GError,
    );
    pub(super) fn frida_device_manager_get_device_by_type_sync(
        manager: *mut FridaDeviceManager,
        device_type: c_int,
        timeout: c_int,
        cancellable: *mut GCancellable,
        error: *mut *mut GError,
    ) -> *mut FridaDevice;
    pub(super) fn frida_device_inject_library_blob_sync(
        device: *mut FridaDevice,
        pid: c_uint,
        blob: *mut GBytes,
        entrypoint: *const c_char,
        data: *const c_char,
        cancellable: *mut GCancellable,
        error: *mut *mut GError,
    ) -> c_uint;
    #[link_name = "_frida_g_main_context_iteration"]
    pub(super) fn g_main_context_iteration(context: *mut GMainContext, may_block: c_int) -> c_int;
    #[link_name = "_frida_g_error_free"]
    pub(super) fn g_error_free(error: *mut GError);
    #[link_name = "_frida_g_bytes_new_static"]
    pub(super) fn g_bytes_new_static(data: *const c_void, size: usize) -> *mut GBytes;
    #[link_name = "_frida_g_bytes_unref"]
    pub(super) fn g_bytes_unref(bytes: *mut GBytes);
}
pub(super) unsafe fn close_manager(manager: *mut FridaDeviceManager) {
    unsafe { frida_device_manager_close_sync(manager, ptr::null_mut(), ptr::null_mut()) };
    unsafe { frida_unref(manager.cast()) };
}

pub(super) unsafe fn pump() {
    let context = unsafe { frida_get_main_context() };
    for _ in 0..64 {
        if unsafe { g_main_context_iteration(context, 0) } == 0 {
            break;
        }
    }
}

pub(super) unsafe fn take_error(error: *mut GError) -> String {
    if error.is_null() {
        return "알 수 없는 Frida Core 오류".to_string();
    }
    let message = unsafe { string_from_pointer((*error).message) };
    unsafe { g_error_free(error) };
    message
}

pub(super) unsafe fn string_from_pointer(value: *const c_char) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}
