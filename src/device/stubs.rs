use std::{
    ffi::{CStr, c_char, c_void},
    os::raw::c_int,
    ptr,
};

const RTLD_NOW: c_int = 2;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

unsafe fn resolve<T: Copy>(name: &CStr) -> Option<T> {
    let handle = unsafe { dlopen(c"libsigchain.so".as_ptr(), RTLD_NOW) };
    if handle.is_null() {
        return None;
    }
    let address = unsafe { dlsym(handle, name.as_ptr()) };
    if address.is_null() {
        return None;
    }
    Some(unsafe { std::mem::transmute_copy(&address) })
}

#[unsafe(no_mangle)]
pub extern "C" fn SetSpecialSignalHandlerFn(signal: c_int, action: *mut c_void) {
    let handler = unsafe {
        resolve::<unsafe extern "C" fn(c_int, *mut c_void)>(c"SetSpecialSignalHandlerFn")
    };
    if let Some(handler) = handler {
        unsafe { handler(signal, action) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn GetSpecialSignalHandlerFn(signal: c_int) -> *mut c_void {
    let handler = unsafe {
        resolve::<unsafe extern "C" fn(c_int) -> *mut c_void>(c"GetSpecialSignalHandlerFn")
    };
    handler
        .map(|handler| unsafe { handler(signal) })
        .unwrap_or(ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "C" fn EnsureFrontOfChain(signal: c_int) {
    let handler = unsafe { resolve::<unsafe extern "C" fn(c_int)>(c"EnsureFrontOfChain") };
    if let Some(handler) = handler {
        unsafe { handler(signal) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn InitializeSignalChain() {
    let handler = unsafe { resolve::<unsafe extern "C" fn()>(c"InitializeSignalChain") };
    if let Some(handler) = handler {
        unsafe { handler() };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AddSpecialSignalHandlerFn(signal: c_int, action: *mut c_void) {
    let handler = unsafe {
        resolve::<unsafe extern "C" fn(c_int, *mut c_void)>(c"AddSpecialSignalHandlerFn")
    };
    if let Some(handler) = handler {
        unsafe { handler(signal, action) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn RemoveSpecialSignalHandlerFn(signal: c_int, action: *mut c_void) {
    let handler = unsafe {
        resolve::<unsafe extern "C" fn(c_int, *mut c_void)>(c"RemoveSpecialSignalHandlerFn")
    };
    if let Some(handler) = handler {
        unsafe { handler(signal, action) };
    }
}
