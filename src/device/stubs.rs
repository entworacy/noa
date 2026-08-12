use std::{ffi::c_void, os::raw::c_int, ptr};

#[unsafe(no_mangle)]
pub extern "C" fn SetSpecialSignalHandlerFn(_: c_int, _: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn GetSpecialSignalHandlerFn(_: c_int) -> *mut c_void {
    ptr::null_mut()
}

#[unsafe(no_mangle)]
pub extern "C" fn EnsureFrontOfChain(_: c_int) {}

#[unsafe(no_mangle)]
pub extern "C" fn InitializeSignalChain() {}

#[unsafe(no_mangle)]
pub extern "C" fn AddSpecialSignalHandlerFn(_: c_int, _: *mut c_void) {}

#[unsafe(no_mangle)]
pub extern "C" fn RemoveSpecialSignalHandlerFn(_: c_int, _: *mut c_void) {}
