use std::{
    ffi::{CStr, c_char, c_int, c_void},
    mem::MaybeUninit,
    ptr,
};

use jni::sys::{JNI_OK, JNI_VERSION_1_6};

const IMMEDIATE_BIND: c_int = 2;

#[repr(C)]
struct InvocationHeader {
    _opaque: [u8; 0],
}

#[repr(C, align(16))]
struct InvocationStorage([MaybeUninit<u8>; 256]);

type VmConstructor = unsafe extern "C" fn(
    *mut *mut jni::sys::JavaVM,
    *mut *mut jni::sys::JNIEnv,
    *mut jni::sys::JavaVMInitArgs,
) -> i32;
type NativeRegistrar = unsafe extern "C" fn(*mut jni::sys::JNIEnv) -> i32;
type LegacyNativeRegistrar = unsafe extern "C" fn(*mut jni::sys::JNIEnv, *mut c_void) -> i32;
type InvocationFactory = unsafe extern "C" fn() -> *mut InvocationHeader;
type InvocationStarter = unsafe extern "C" fn(*mut InvocationHeader, *const c_char) -> c_int;
type LegacyInvocationConstructor = unsafe extern "C" fn(*mut InvocationHeader);
type LegacyInvocationStarter = unsafe extern "C" fn(*mut InvocationHeader, *const c_char) -> bool;

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

struct SharedObject(*mut c_void);

impl SharedObject {
    unsafe fn acquire(path: &CStr) -> Result<Self, String> {
        let handle = unsafe { dlopen(path.as_ptr(), IMMEDIATE_BIND) };
        if handle.is_null() {
            Err(loader_failure())
        } else {
            Ok(Self(handle))
        }
    }

    unsafe fn require<T: Copy>(&self, name: &CStr) -> Result<T, String> {
        let address = unsafe { dlsym(self.0, name.as_ptr()) };
        if address.is_null() {
            return Err(loader_failure());
        }
        Ok(unsafe { std::mem::transmute_copy(&address) })
    }

    unsafe fn probe<T: Copy>(&self, name: &CStr) -> Option<T> {
        unsafe { self.require(name).ok() }
    }
}

fn loader_failure() -> String {
    let error = unsafe { dlerror() };
    if error.is_null() {
        "동적 링커 오류".to_string()
    } else {
        unsafe { CStr::from_ptr(error).to_string_lossy().into_owned() }
    }
}

pub struct RuntimeVm;

impl RuntimeVm {
    pub unsafe fn launch() -> Result<jni::JavaVM, String> {
        let runtime_name = c"libandroid_runtime.so";
        let art_name = c"libart.so";
        let runtime = unsafe { SharedObject::acquire(runtime_name) }
            .map_err(|error| format!("Android 런타임 로드 실패: {error}"))?;
        unsafe { prepare_invocation(&runtime, art_name) }?;
        let mut options = jni::sys::JavaVMInitArgs {
            version: JNI_VERSION_1_6,
            nOptions: 0,
            options: ptr::null_mut(),
            ignoreUnrecognized: false,
        };
        let mut vm_pointer = ptr::null_mut();
        let mut environment_pointer = ptr::null_mut();
        let constructor: VmConstructor = unsafe { runtime.require(c"JNI_CreateJavaVM") }?;
        let status =
            unsafe { constructor(&mut vm_pointer, &mut environment_pointer, &mut options) };
        if status != JNI_OK || vm_pointer.is_null() || environment_pointer.is_null() {
            return Err(format!("ART VM 생성 실패: {status}"));
        }
        unsafe { register_android_classes(&runtime, environment_pointer) }?;
        Ok(unsafe { jni::JavaVM::from_raw(vm_pointer) })
    }
}

unsafe fn prepare_invocation(runtime: &SharedObject, art_name: &CStr) -> Result<(), String> {
    let factory: Option<InvocationFactory> = unsafe { runtime.probe(c"JniInvocationCreate") };
    let starter: Option<InvocationStarter> = unsafe { runtime.probe(c"JniInvocationInit") };
    if let (Some(factory), Some(starter)) = (factory, starter) {
        let instance = unsafe { factory() };
        if instance.is_null() || unsafe { starter(instance, art_name.as_ptr()) } == 0 {
            return Err("JNI 호출 계층 초기화 실패".to_string());
        }
        return Ok(());
    }

    let construct: LegacyInvocationConstructor =
        unsafe { runtime.require(c"_ZN13JniInvocationC1Ev") }?;
    let start: LegacyInvocationStarter =
        unsafe { runtime.require(c"_ZN13JniInvocation4InitEPKc") }?;
    let storage = Box::new(InvocationStorage([MaybeUninit::uninit(); 256]));
    let instance = Box::into_raw(storage).cast::<InvocationHeader>();
    unsafe { construct(instance) };
    if !unsafe { start(instance, ptr::null()) } && !unsafe { start(instance, art_name.as_ptr()) } {
        return Err("JNI 호출 계층 초기화 실패".to_string());
    }
    Ok(())
}

unsafe fn register_android_classes(
    runtime: &SharedObject,
    environment: *mut jni::sys::JNIEnv,
) -> Result<(), String> {
    let result = if let Some(register) =
        unsafe { runtime.probe::<NativeRegistrar>(c"registerFrameworkNatives") }
    {
        unsafe { register(environment) }
    } else {
        let register: LegacyNativeRegistrar = unsafe {
            runtime.require(c"Java_com_android_internal_util_WithFramework_registerNatives")
        }?;
        unsafe { register(environment, ptr::null_mut()) }
    };
    if result == JNI_OK {
        Ok(())
    } else {
        Err(format!("Android 네이티브 클래스 등록 실패: {result}"))
    }
}
