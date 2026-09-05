//! LSPlant ABI and in-memory library loading. Handles remain live for process lifetime.
use jni::sys::{JNIEnv, jobject};
use std::{
    ffi::{CStr, c_char, c_int, c_void},
    fs::File,
    io::Write,
    os::fd::FromRawFd,
};

unsafe extern "C" {
    fn dlerror() -> *const c_char;
    fn noa_dlopen_fd(fd: c_int, flags: c_int) -> *mut c_void;
    pub fn noa_lsplant_init(env: *mut JNIEnv, handle: *mut c_void) -> bool;
    pub fn noa_lsplant_last_error() -> *const c_char;
    pub fn noa_lsplant_uses_shorty_fallback() -> bool;
    pub fn noa_lsplant_hook(
        env: *mut JNIEnv,
        handle: *mut c_void,
        target: jobject,
        hooker: jobject,
        callback: jobject,
    ) -> jobject;
    pub fn noa_lsplant_deoptimize(env: *mut JNIEnv, handle: *mut c_void, target: jobject) -> bool;
}

pub fn load(blob: &[u8], name: &std::ffi::CStr) -> Result<*mut c_void, String> {
    let fd =
        unsafe { libc::syscall(libc::SYS_memfd_create, name.as_ptr(), libc::MFD_CLOEXEC) } as c_int;
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(blob).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    let handle = unsafe { noa_dlopen_fd(fd, libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        let detail = unsafe {
            let value = dlerror();
            if value.is_null() {
                "unknown dlopen error".to_string()
            } else {
                CStr::from_ptr(value).to_string_lossy().into_owned()
            }
        };
        Err(format!("LSPlant load failed: {detail}"))
    } else {
        Ok(handle)
    }
}

pub fn initialization_error() -> String {
    let detail = unsafe {
        let value = noa_lsplant_last_error();
        (!value.is_null()).then(|| CStr::from_ptr(value).to_string_lossy().into_owned())
    }
    .unwrap_or_else(|| "unknown LSPlant initialization error".to_string());
    format!("LSPlant initialization failed: {detail}")
}
