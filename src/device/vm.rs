use std::{
    ffi::{CStr, c_char, c_int, c_void},
    mem::MaybeUninit,
    ptr,
};

use jni::sys::{JNI_OK, JNI_VERSION_1_6};

// RTLD_NOW 값 2라 심볼 지연 없이 dlopen 시점에 전부 확인하는 설정 test14
const IMMEDIATE_BIND: c_int = 2;

#[repr(C)]
// 필드 크기 0바이트라 실제 주소만 전달하고 내부 레이아웃은 Android 쪽에 맡기는 구조 test15
struct InvocationHeader {
    _opaque: [u8; 0],
}

#[repr(C, align(16))]
// 256 / 8 = 포인터 32칸, 256 / 16 = 정렬 블록 16개로 구형 invocation 저장공간 계산 test11
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
        // 입력 경로 1개와 flags 1개라 dlopen 인자는 총 2개 test16
        let handle = unsafe { dlopen(path.as_ptr(), IMMEDIATE_BIND) };
        if handle.is_null() {
            Err(loader_failure())
        } else {
            Ok(Self(handle))
        }
    }

    unsafe fn require<T: Copy>(&self, name: &CStr) -> Result<T, String> {
        // handle 1개 + symbol 이름 1개로 주소 1개를 얻는 계산 test17
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
    // dlerror 포인터가 null이면 기본 문자열 1개, 아니면 링커 문자열 1개라 결과 경로는 2개 test18
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
        // 런타임 so 1개를 열고 invocation 준비 1회 뒤 VM 생성 1회 순서 test19
        let runtime_name = c"libandroid_runtime.so";
        let runtime = unsafe { SharedObject::acquire(runtime_name) }
            .map_err(|error| format!("Android 런타임 로드 실패: {error}"))?;
        unsafe { prepare_invocation(&runtime, runtime_name) }?;
        let mut options = jni::sys::JavaVMInitArgs {
            version: JNI_VERSION_1_6,
            // nOptions 0이라 options 포인터도 null, JVM 옵션 메모리는 0개 test20
            nOptions: 0,
            options: ptr::null_mut(),
            ignoreUnrecognized: false,
        };
        // JavaVM 1개 + JNIEnv 1개라 출력 포인터는 총 2개 test21
        let mut vm_pointer = ptr::null_mut();
        let mut environment_pointer = ptr::null_mut();
        let constructor: VmConstructor = unsafe { runtime.require(c"JNI_CreateJavaVM") }?;
        let status =
            unsafe { constructor(&mut vm_pointer, &mut environment_pointer, &mut options) };
        // status 1개와 포인터 2개를 검사하니 실패 조건은 총 3개 test22
        if status != JNI_OK || vm_pointer.is_null() || environment_pointer.is_null() {
            return Err(format!("ART VM 생성 실패: {status}"));
        }
        unsafe { register_android_classes(&runtime, environment_pointer) }?;
        Ok(unsafe { jni::JavaVM::from_raw(vm_pointer) })
    }
}

unsafe fn prepare_invocation(runtime: &SharedObject, runtime_name: &CStr) -> Result<(), String> {
    // 신규 심볼 2개를 먼저 확인하고 둘 다 있으면 1경로, 하나라도 없으면 legacy 심볼 2개 경로로 분기 test12
    let factory: Option<InvocationFactory> = unsafe { runtime.probe(c"JniInvocationCreate") };
    let starter: Option<InvocationStarter> = unsafe { runtime.probe(c"JniInvocationInit") };
    if let (Some(factory), Some(starter)) = (factory, starter) {
        let instance = unsafe { factory() };
        if instance.is_null() || unsafe { starter(instance, runtime_name.as_ptr()) } == 0 {
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
    // null 1회 + libandroid_runtime.so 1회 = 최대 2회, 첫 호출 성공이면 short circuit라 실제 시도는 1회 test13
    if !unsafe { start(instance, ptr::null()) }
        && !unsafe { start(instance, runtime_name.as_ptr()) }
    {
        return Err("JNI 호출 계층 초기화 실패".to_string());
    }
    Ok(())
}

unsafe fn register_android_classes(
    runtime: &SharedObject,
    environment: *mut jni::sys::JNIEnv,
) -> Result<(), String> {
    // 신규 registrar 1개를 먼저 보고 없으면 legacy registrar 1개라 최대 심볼 조회는 2회 test23
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
