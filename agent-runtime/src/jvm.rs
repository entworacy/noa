use jni::sys::{JNI_EDETACHED, JNI_OK, JNI_VERSION_1_6, JNIEnv, JavaVM};
use std::{
    ffi::{c_char, c_void},
    ptr,
};

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
type GetCreatedVms = unsafe extern "system" fn(*mut *mut JavaVM, i32, *mut i32) -> i32;

/// Run on an attached JVM thread, detaching only when this call attached it.
/// # Safety
/// `vm` must identify a live JVM for the entire call.
pub unsafe fn with_attached<T>(
    vm: *mut JavaVM,
    run: impl FnOnce(*mut JNIEnv) -> Result<T, String>,
) -> Result<T, String> {
    unsafe {
        let functions = &**vm;
        let mut raw = ptr::null_mut();
        let mut detach = false;
        let status = (functions.v1_4.GetEnv)(vm, &mut raw, JNI_VERSION_1_6);
        if status == JNI_EDETACHED {
            let attached = (functions.v1_4.AttachCurrentThread)(vm, &mut raw, ptr::null_mut());
            if attached != JNI_OK {
                return Err(format!("AttachCurrentThread failed: {attached}"));
            }
            detach = true;
        } else if status != JNI_OK {
            return Err(format!("GetEnv failed: {status}"));
        }
        let result = run(raw.cast());
        if detach {
            let _ = (functions.v1_4.DetachCurrentThread)(vm);
        }
        result
    }
}

/// Find the JVM exported by the current process.
/// # Safety
/// The process must provide the Android JNI invocation ABI.
pub unsafe fn locate_vm() -> Result<*mut JavaVM, String> {
    let address = unsafe { dlsym(ptr::null_mut(), c"JNI_GetCreatedJavaVMs".as_ptr()) };
    if address.is_null() {
        return Err("JNI_GetCreatedJavaVMs was not found".to_string());
    }
    let get_vms: GetCreatedVms = unsafe { std::mem::transmute(address) };
    let mut vm = ptr::null_mut();
    let mut count = 0;
    let status = unsafe { get_vms(&mut vm, 1, &mut count) };
    if status != JNI_OK || count < 1 || vm.is_null() {
        return Err(format!(
            "JNI_GetCreatedJavaVMs failed: {status}, count={count}"
        ));
    }
    Ok(vm)
}
