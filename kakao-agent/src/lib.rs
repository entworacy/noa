use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    fs::File,
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    os::fd::FromRawFd,
    ptr,
    sync::{Once, OnceLock},
    thread,
    time::Duration,
};

use jni::sys::{
    JNI_EDETACHED, JNI_OK, JNI_VERSION_1_6, JNIEnv, JNINativeMethod, JavaVM, jboolean, jclass,
    jint, jlong, jobject, jobjectArray, jstring, jvalue,
};
use serde::{Deserialize, Serialize};

mod actions;
mod commands;
mod events;
mod packets;
mod room;
mod signature_api;
mod signature_index;
mod vox;

pub(crate) use commands::{
    CommandResult, OpenChatJoinResult, Operation, command_operation, mark_complete, mark_loaded,
};
pub(crate) use signature_api::{
    invoke_signature_operation, signature_class, signature_object, signature_static_value,
};

const ADAPTER_DEX: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../assets/noa-kakao-agent.dex"
));
const ACTION_SEND: i32 = 1;
const ACTION_KICK: i32 = 2;
const ACTION_CHATONROOM: i32 = 3;
const ACTION_LOAD_OPEN_CHAT_MEMBER: i32 = 4;
const ACTION_INSTALL_ROOM_WATCHER: i32 = 5;
const ACTION_VOX: i32 = 6;
const ACTION_HIDE_MESSAGE: i32 = 7;
const KIND_LOCO_SEND: i32 = 1;
const KIND_LOCO_RECEIVE: i32 = 2;
const LOG_INFO: c_int = 4;
const LOG_ERROR: c_int = 6;

static START: Once = Once::new();
static EVENT_START: Once = Once::new();
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static RUNTIME_READY: OnceLock<Result<(), String>> = OnceLock::new();

const LSPLANT: &[u8] = include_bytes!(env!("NOA_LSPLANT_BLOB"));

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    fn dlerror() -> *const c_char;
    fn noa_dlopen_fd(fd: c_int, flags: c_int) -> *mut c_void;
    fn noa_lsplant_init(env: *mut JNIEnv, handle: *mut c_void) -> bool;
    fn noa_lsplant_last_error() -> *const c_char;
    fn noa_lsplant_uses_shorty_fallback() -> bool;
    fn noa_lsplant_hook(
        env: *mut JNIEnv,
        handle: *mut c_void,
        target: jobject,
        hooker: jobject,
        callback: jobject,
    ) -> jobject;
    fn noa_lsplant_deoptimize(env: *mut JNIEnv, handle: *mut c_void, target: jobject) -> bool;
}

type GetCreatedVms = unsafe extern "system" fn(*mut *mut JavaVM, i32, *mut i32) -> i32;

#[derive(Deserialize)]
struct Bootstrap {
    port: u16,
    event_port: u16,
    token: String,
}

#[derive(Serialize)]
struct Response<'a> {
    id: u64,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<&'a str>,
}

#[derive(Serialize)]
struct Hello<'a> {
    event: &'static str,
    token: &'a str,
    pid: u32,
    protocol: u8,
}

#[derive(Serialize)]
struct FailureHello<'a> {
    event: &'static str,
    token: &'a str,
    pid: u32,
    protocol: u8,
    error: &'a str,
    retryable: bool,
}

struct Runtime {
    vm: usize,
    loader: usize,
    lsplant: usize,
}

struct LocoHooks {
    send: jobject,
    receive: jobject,
    resume: jobject,
    description: String,
}

unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

#[unsafe(no_mangle)]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn noa_agent_main(data: *const c_char, stay_resident: *mut c_int) {
    if !stay_resident.is_null() {
        unsafe { *stay_resident = 1 };
    }
    if data.is_null() {
        return;
    }
    let text = unsafe { CStr::from_ptr(data) }
        .to_string_lossy()
        .into_owned();
    let Ok(config) = serde_json::from_str::<Bootstrap>(&text) else {
        log(LOG_ERROR, "invalid bootstrap payload");
        return;
    };
    START.call_once(|| {
        let _ = thread::Builder::new()
            .name("noa-kakao-agent".to_string())
            .spawn(move || serve(config));
    });
}

fn serve(config: Bootstrap) {
    let address = SocketAddr::from(([127, 0, 0, 1], config.port));
    loop {
        match TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
            Ok(mut stream) => {
                if let Err(error) = session(&mut stream, &config) {
                    log(LOG_ERROR, &error);
                    if initialization_failed() {
                        log(
                            LOG_ERROR,
                            "Kakao agent initialization is permanently stopped for this injection",
                        );
                        return;
                    }
                    thread::sleep(Duration::from_secs(2));
                }
            }
            Err(_) => thread::sleep(Duration::from_secs(1)),
        }
    }
}

fn session(stream: &mut TcpStream, config: &Bootstrap) -> Result<(), String> {
    if let Err(error) = initialize_runtime() {
        let retryable = !initialization_failed();
        let failure = FailureHello {
            event: "error",
            token: &config.token,
            pid: std::process::id(),
            protocol: 1,
            error: &error,
            retryable,
        };
        let _ = write_json(stream, &failure);
        return Err(error);
    }
    ensure_event_bridge(config);
    stream
        .set_nodelay(true)
        .map_err(|error| error.to_string())?;
    let hello = Hello {
        event: "ready",
        token: &config.token,
        pid: std::process::id(),
        protocol: 1,
    };
    write_json(stream, &hello)?;
    let reader_stream = stream.try_clone().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(reader_stream);
    loop {
        let mut line = String::new();
        if reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?
            == 0
        {
            return Err("Noa bridge disconnected".to_string());
        }
        let request = match serde_json::from_str::<commands::Request>(line.trim()) {
            Ok(request) => request,
            Err(error) => {
                write_response(stream, 0, Err(format!("invalid command: {error}")))?;
                continue;
            }
        };
        if request.token != config.token {
            write_response(stream, request.id, Err("authentication failed".to_string()))?;
            continue;
        }
        let result = commands::execute(request);
        write_response(stream, result.0, result.1)?;
    }
}

fn ensure_event_bridge(config: &Bootstrap) {
    let event_address = SocketAddr::from(([127, 0, 0, 1], config.event_port));
    let event_token = config.token.clone();
    EVENT_START.call_once(move || {
        let (event_sender, event_receiver) = events::channel();
        events::install(event_sender);
        let _ = thread::Builder::new()
            .name("noa-kakao-events".to_string())
            .spawn(move || events::run(event_address, event_token, event_receiver));
    });
}

fn initialization_failed() -> bool {
    RUNTIME_READY.get().is_some_and(Result::is_err)
}

fn initialize_runtime() -> Result<(), String> {
    if let Some(result) = RUNTIME_READY.get() {
        return result.clone();
    }

    let runtime = bootstrap_runtime()?;

    RUNTIME_READY
        .get_or_init(|| {
            with_attached(runtime.vm as *mut JavaVM, |env| unsafe {
                log(LOG_INFO, "Kakao agent initialization: initializing LSPlant");
                if !noa_lsplant_init(env, runtime.lsplant as *mut c_void) {
                    return Err(lsplant_initialization_error());
                }
                if noa_lsplant_uses_shorty_fallback() {
                    log(
                        LOG_INFO,
                        "LSPlant ART GetMethodShorty compatibility fallback active",
                    );
                }
                log(
                    LOG_INFO,
                    "Kakao agent initialization: resolving LOCO signature",
                );
                let signature_description = signature_api::verify_discovery(env)
                    .map_err(|error| format!("verify KakaoTalk signatures: {error}"))?;
                log(
                    LOG_INFO,
                    &format!("KakaoTalk signature discovery verified: {signature_description}"),
                );
                let hooks = resolve_loco_hooks(env)
                    .map_err(|error| format!("resolve LOCO signature: {error}"))?;
                log(
                    LOG_INFO,
                    &format!("LOCO signature resolved: {}", hooks.description),
                );
                log(LOG_INFO, "Kakao agent initialization: hooking LOCO send");
                install_hook(env, hooks.send, "LOCO send", KIND_LOCO_SEND, false, 1)
                    .map_err(|error| format!("hook LOCO send: {error}"))?;
                log(LOG_INFO, "Kakao agent initialization: hooking LOCO receive");
                install_hook(
                    env,
                    hooks.receive,
                    "LOCO receive",
                    KIND_LOCO_RECEIVE,
                    false,
                    1,
                )
                .map_err(|error| format!("hook LOCO receive: {error}"))?;
                match vox::install_hook(env) {
                    Ok(()) => log(LOG_INFO, "VOX WebRTC audio injection hook ready"),
                    Err(error) => log(
                        LOG_ERROR,
                        &format!("VOX audio hook unavailable; chat hooks remain active: {error}"),
                    ),
                }
                log(
                    LOG_INFO,
                    "Kakao agent initialization: deoptimizing LOCO coroutine",
                );
                deoptimize_method(env, hooks.resume, "LOCO coroutine resume")
                    .map_err(|error| format!("deoptimize LOCO coroutine: {error}"))?;
                log(
                    LOG_INFO,
                    "Kakao agent initialization: installing room watcher",
                );
                post_main(env, 0, ACTION_INSTALL_ROOM_WATCHER)
                    .map_err(|error| format!("install room watcher: {error}"))?;
                Ok(())
            })
        })
        .clone()?;
    log(LOG_INFO, "Rust KakaoTalk agent ready");
    Ok(())
}

fn lsplant_initialization_error() -> String {
    let detail = unsafe {
        let value = noa_lsplant_last_error();
        (!value.is_null()).then(|| CStr::from_ptr(value).to_string_lossy().into_owned())
    }
    .unwrap_or_else(|| "unknown LSPlant initialization error".to_string());
    format!("LSPlant initialization failed: {detail}")
}

fn bootstrap_runtime() -> Result<&'static Runtime, String> {
    if let Some(runtime) = RUNTIME.get() {
        return Ok(runtime);
    }
    log(LOG_INFO, "Kakao agent initialization: locating JVM");
    let vm = unsafe { locate_vm() }.map_err(|error| format!("locate JVM: {error}"))?;
    log(LOG_INFO, "Kakao agent initialization: loading LSPlant");
    let lsplant = load_lsplant().map_err(|error| format!("load LSPlant: {error}"))?;
    log(LOG_INFO, "Kakao agent initialization: waiting for Android application");
    let loader = with_attached(vm, |env| unsafe { create_loader(env) })
        .map_err(|error| format!("create DEX adapter: {error}"))?;
    let _ = RUNTIME.set(Runtime {
        vm: vm as usize,
        loader: loader as usize,
        lsplant: lsplant as usize,
    });
    RUNTIME
        .get()
        .ok_or_else(|| "native runtime could not be stored".to_string())
}

fn with_env<T>(run: impl FnOnce(*mut JNIEnv) -> Result<T, String>) -> Result<T, String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "native runtime is not initialized".to_string())?;
    with_attached(runtime.vm as *mut JavaVM, run)
}

fn with_attached<T>(
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

unsafe fn locate_vm() -> Result<*mut JavaVM, String> {
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

unsafe fn create_loader(env: *mut JNIEnv) -> Result<jobject, String> {
    let activity_thread = unsafe { find_class(env, "android/app/ActivityThread")? };
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let application = loop {
        let application = unsafe {
            call_static_object(
                env,
                activity_thread,
                "currentApplication",
                "()Landroid/app/Application;",
                &[],
            )?
        };
        if !application.is_null() {
            break application;
        }
        if std::time::Instant::now() >= deadline {
            return Err("Android application is not ready after 20 seconds".to_string());
        }
        thread::sleep(Duration::from_millis(100));
    };
    let parent = unsafe {
        call_object(
            env,
            application,
            "getClassLoader",
            "()Ljava/lang/ClassLoader;",
            &[],
        )?
    };
    let buffer = unsafe {
        ((**env).v1_4.NewDirectByteBuffer)(
            env,
            ADAPTER_DEX.as_ptr().cast_mut().cast(),
            ADAPTER_DEX.len() as jlong,
        )
    };
    unsafe { check(env, "create DEX buffer")? };
    let dex_class = unsafe { find_class(env, "dalvik/system/InMemoryDexClassLoader")? };
    let loader = unsafe {
        new_object(
            env,
            dex_class,
            "(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V",
            &[object_value(buffer), object_value(parent)],
        )?
    };
    let global = unsafe { ((**env).v1_4.NewGlobalRef)(env, loader) };
    unsafe { check(env, "retain DEX class loader")? };
    if global.is_null() {
        return Err("DEX class loader global reference is null".to_string());
    }
    if let Err(error) = unsafe { initialize_loader(env, global) } {
        unsafe { ((**env).v1_4.DeleteGlobalRef)(env, global) };
        return Err(error);
    }
    Ok(global)
}

unsafe fn initialize_loader(env: *mut JNIEnv, loader: jobject) -> Result<(), String> {
    let bridge = unsafe { load_class(env, loader, "dev.noa.kakao.Bridge")? };
    let methods = [
        native_method("loaded", "(J)V", bridge_loaded as *mut c_void),
        native_method(
            "complete",
            "(JZLjava/lang/String;)V",
            bridge_complete as *mut c_void,
        ),
        native_method("dispatch", "(JI)V", bridge_dispatch as *mut c_void),
        native_method(
            "capture",
            "(ILjava/lang/Object;)V",
            bridge_capture as *mut c_void,
        ),
        native_method(
            "databaseInvalidated",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            room::invalidated as *mut c_void,
        ),
        native_method(
            "processVoxAudio",
            "(Ljava/nio/ByteBuffer;I)V",
            vox::process_audio as *mut c_void,
        ),
    ];
    let status = unsafe {
        ((**env).v1_4.RegisterNatives)(env, bridge, methods.as_ptr(), methods.len() as jint)
    };
    unsafe { check(env, "register native callbacks")? };
    if status != JNI_OK {
        return Err(format!("RegisterNatives failed: {status}"));
    }
    for name in [
        "dev.noa.kakao.LoadContinuation",
        "dev.noa.kakao.MainDispatch",
        "dev.noa.kakao.Hooker",
        "dev.noa.kakao.RoomWatcher",
        "dev.noa.kakao.VoxAudioHooker",
        "dev.noa.kakao.VoxController",
    ] {
        unsafe { load_class(env, loader, name)? };
    }
    let resolver = unsafe { load_class(env, loader, "dev.noa.kakao.KakaoSignatureResolver")? };
    let description = unsafe { signature_index::install(env, resolver)? };
    log(
        LOG_INFO,
        &format!("Rust DEX signature index installed: {description}"),
    );
    Ok(())
}

unsafe extern "system" fn bridge_loaded(_: *mut JNIEnv, _: jclass, id: jlong) {
    mark_loaded(id as u64);
}

unsafe extern "system" fn bridge_complete(
    env: *mut JNIEnv,
    _: jclass,
    id: jlong,
    ok: jboolean,
    error: jstring,
) {
    let result = if ok {
        Ok(())
    } else if error.is_null() {
        Err("KakaoTalk native callback failed".to_string())
    } else {
        Err(unsafe { java_string(env, error) }
            .unwrap_or_else(|_| "KakaoTalk native callback failed".to_string()))
    };
    mark_complete(id as u64, result);
}

unsafe extern "system" fn bridge_dispatch(env: *mut JNIEnv, _: jclass, id: jlong, action: jint) {
    let result = match action {
        ACTION_SEND => unsafe { actions::dispatch_send(env, id as u64) },
        ACTION_KICK => unsafe { actions::dispatch_kick(env, id as u64) },
        ACTION_HIDE_MESSAGE => unsafe { actions::dispatch_hide_message(env, id as u64) },
        ACTION_CHATONROOM | ACTION_LOAD_OPEN_CHAT_MEMBER => {
            command_operation(id as u64).and_then(|operation| unsafe {
                commands::dispatch_packet_action(env, id as u64, operation)
            })
        }
        ACTION_INSTALL_ROOM_WATCHER => unsafe { room::install(env) },
        ACTION_VOX => command_operation(id as u64)
            .and_then(|operation| unsafe { vox::dispatch_main(env, id as u64, operation) }),
        _ => Err("unknown main-thread action".to_string()),
    };
    if action == ACTION_INSTALL_ROOM_WATCHER {
        match result {
            Ok(()) => log(LOG_INFO, "Room invalidation watcher ready"),
            Err(error) => log(
                LOG_ERROR,
                &format!("Room watcher installation failed: {error}"),
            ),
        }
    } else if let Err(error) = result {
        mark_complete(id as u64, Err(error));
    }
}

unsafe extern "system" fn bridge_capture(env: *mut JNIEnv, _: jclass, kind: jint, packet: jobject) {
    unsafe { events::capture(env, kind, packet) };
}

unsafe fn post_main(env: *mut JNIEnv, id: u64, action: i32) -> Result<(), String> {
    let runnable_class = unsafe { app_class(env, "dev.noa.kakao.MainDispatch")? };
    let runnable = unsafe {
        new_object(
            env,
            runnable_class,
            "(JI)V",
            &[long_value(id as i64), int_value(action)],
        )?
    };
    let looper_class = unsafe { find_class(env, "android/os/Looper")? };
    let looper = unsafe {
        call_static_object(
            env,
            looper_class,
            "getMainLooper",
            "()Landroid/os/Looper;",
            &[],
        )?
    };
    let handler_class = unsafe { find_class(env, "android/os/Handler")? };
    let handler = unsafe {
        new_object(
            env,
            handler_class,
            "(Landroid/os/Looper;)V",
            &[object_value(looper)],
        )?
    };
    let posted = unsafe {
        call_boolean(
            env,
            handler,
            "post",
            "(Ljava/lang/Runnable;)Z",
            &[object_value(runnable)],
        )?
    };
    if posted {
        Ok(())
    } else {
        Err("Android main thread rejected the command".to_string())
    }
}

unsafe fn loco_connected(env: *mut JNIEnv) -> Result<bool, String> {
    let resolver = unsafe { app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    unsafe { call_static_boolean(env, resolver, "locoConnected", "()Z", &[]) }
}

unsafe fn find_room(env: *mut JNIEnv, room: i64) -> Result<jobject, String> {
    let resolver = unsafe { app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    let result = unsafe {
        call_static_object(
            env,
            resolver,
            "findRoom",
            "(J)Ljava/lang/Object;",
            &[long_value(room)],
        )?
    };
    if result.is_null() {
        Err(format!("chat room not found: {room}"))
    } else {
        Ok(result)
    }
}

unsafe fn install_hook(
    env: *mut JNIEnv,
    target: jobject,
    label: &str,
    kind: i32,
    static_target: bool,
    packet_index: i32,
) -> Result<(), String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "runtime is unavailable".to_string())?;
    let hooker_class = unsafe { app_class(env, "dev.noa.kakao.Hooker")? };
    let hooker = unsafe {
        new_object(
            env,
            hooker_class,
            "(IZI)V",
            &[
                int_value(kind),
                bool_value(static_target),
                int_value(packet_index),
            ],
        )?
    };
    let callback =
        unsafe { find_exact_method(env, hooker_class, "callback", &["[Ljava.lang.Object;"])? };
    let backup = unsafe {
        noa_lsplant_hook(
            env,
            runtime.lsplant as *mut c_void,
            target,
            hooker,
            callback,
        )
    };
    unsafe { check(env, "install LOCO hook")? };
    if backup.is_null() {
        return Err(format!("LSPlant returned no backup for {label}"));
    }
    let field = unsafe {
        ((**env).v1_4.GetFieldID)(
            env,
            hooker_class,
            c"backup".as_ptr(),
            c"Ljava/lang/reflect/Method;".as_ptr(),
        )
    };
    unsafe { check(env, "resolve Hooker.backup")? };
    unsafe { ((**env).v1_4.SetObjectField)(env, hooker, field, backup) };
    unsafe { check(env, "store Hooker.backup")? };
    let retained = unsafe { ((**env).v1_4.NewGlobalRef)(env, hooker) };
    unsafe { check(env, "retain LOCO hook")? };
    if retained.is_null() {
        Err("hooker global reference is null".to_string())
    } else {
        Ok(())
    }
}

unsafe fn deoptimize_method(env: *mut JNIEnv, target: jobject, label: &str) -> Result<(), String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "runtime is unavailable".to_string())?;
    if unsafe { noa_lsplant_deoptimize(env, runtime.lsplant as *mut c_void, target) } {
        Ok(())
    } else {
        Err(format!("LSPlant could not deoptimize {label}"))
    }
}

unsafe fn resolve_loco_hooks(env: *mut JNIEnv) -> Result<LocoHooks, String> {
    let resolver = unsafe { app_class(env, "dev.noa.kakao.LocoSignatureResolver")? };
    let resolved = unsafe {
        call_static_object(env, resolver, "resolve", "()[Ljava/lang/Object;", &[])? as jobjectArray
    };
    if resolved.is_null() {
        return Err("LOCO signature resolver returned null".to_string());
    }
    let count = unsafe { ((**env).v1_4.GetArrayLength)(env, resolved) };
    unsafe { check(env, "read LOCO signature result")? };
    if count != 4 {
        return Err(format!(
            "LOCO signature resolver returned {count} values instead of 4"
        ));
    }
    let mut values = [ptr::null_mut(); 4];
    for (index, value) in values.iter_mut().enumerate() {
        *value = unsafe { ((**env).v1_4.GetObjectArrayElement)(env, resolved, index as jint) };
        unsafe { check(env, "read LOCO signature value")? };
        if value.is_null() {
            return Err(format!("LOCO signature value {index} was null"));
        }
    }
    Ok(LocoHooks {
        send: values[0],
        receive: values[1],
        resume: values[2],
        description: unsafe { java_string(env, values[3].cast())? },
    })
}

unsafe fn find_exact_method(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    parameter_types: &[&str],
) -> Result<jobject, String> {
    let methods = unsafe {
        call_object(
            env,
            class,
            "getDeclaredMethods",
            "()[Ljava/lang/reflect/Method;",
            &[],
        )? as jobjectArray
    };
    let count = unsafe { ((**env).v1_4.GetArrayLength)(env, methods) };
    unsafe { check(env, "read declared methods")? };
    for index in 0..count {
        let method = unsafe { ((**env).v1_4.GetObjectArrayElement)(env, methods, index) };
        unsafe { check(env, "read declared method")? };
        let method_name =
            unsafe { call_object(env, method, "getName", "()Ljava/lang/String;", &[])? };
        if unsafe { java_string(env, method_name.cast())? } != name {
            continue;
        }
        let parameters = unsafe {
            call_object(
                env,
                method,
                "getParameterTypes",
                "()[Ljava/lang/Class;",
                &[],
            )? as jobjectArray
        };
        let parameter_count = unsafe { ((**env).v1_4.GetArrayLength)(env, parameters) };
        if parameter_count as usize != parameter_types.len() {
            continue;
        }
        let mut matched = true;
        for (position, expected) in parameter_types.iter().enumerate() {
            let parameter =
                unsafe { ((**env).v1_4.GetObjectArrayElement)(env, parameters, position as i32) };
            let actual =
                unsafe { call_object(env, parameter, "getName", "()Ljava/lang/String;", &[])? };
            if unsafe { java_string(env, actual.cast())? } != *expected {
                matched = false;
                break;
            }
        }
        if matched {
            unsafe { call_void(env, method, "setAccessible", "(Z)V", &[bool_value(true)])? };
            return Ok(method);
        }
    }
    Err(format!(
        "method {name}({}) was not found",
        parameter_types.join(", ")
    ))
}

unsafe fn app_class(env: *mut JNIEnv, name: &str) -> Result<jclass, String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "native runtime is not initialized".to_string())?;
    unsafe { load_class(env, runtime.loader as jobject, name) }
}

unsafe fn load_class(env: *mut JNIEnv, loader: jobject, name: &str) -> Result<jclass, String> {
    let name = unsafe { new_string(env, name)? };
    let class = unsafe {
        call_object(
            env,
            loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[object_value(name)],
        )?
    };
    if class.is_null() {
        Err("class loader returned null".to_string())
    } else {
        Ok(class.cast())
    }
}

unsafe fn static_field(env: *mut JNIEnv, class: jclass, name: &str) -> Result<jobject, String> {
    let name = unsafe { new_string(env, name)? };
    let field = unsafe {
        call_object(
            env,
            class,
            "getDeclaredField",
            "(Ljava/lang/String;)Ljava/lang/reflect/Field;",
            &[object_value(name)],
        )?
    };
    unsafe { call_void(env, field, "setAccessible", "(Z)V", &[bool_value(true)])? };
    unsafe {
        call_object(
            env,
            field,
            "get",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            &[object_value(ptr::null_mut())],
        )
    }
}

unsafe fn instance_field(env: *mut JNIEnv, target: jobject, name: &str) -> Result<jobject, String> {
    let mut class = unsafe { ((**env).v1_4.GetObjectClass)(env, target) };
    unsafe { check(env, "resolve instance class")? };
    while !class.is_null() {
        let field_name = unsafe { new_string(env, name)? };
        let field = unsafe {
            call_object(
                env,
                class,
                "getDeclaredField",
                "(Ljava/lang/String;)Ljava/lang/reflect/Field;",
                &[object_value(field_name)],
            )
        };
        match field {
            Ok(field) => {
                unsafe { call_void(env, field, "setAccessible", "(Z)V", &[bool_value(true)])? };
                return unsafe {
                    call_object(
                        env,
                        field,
                        "get",
                        "(Ljava/lang/Object;)Ljava/lang/Object;",
                        &[object_value(target)],
                    )
                };
            }
            Err(_) => {
                class = unsafe {
                    call_object(env, class, "getSuperclass", "()Ljava/lang/Class;", &[])?
                }
                .cast();
            }
        }
    }
    Err(format!("instance field {name} was not found"))
}

unsafe fn instance_field_by_type(
    env: *mut JNIEnv,
    target: jobject,
    type_name: &str,
) -> Result<jobject, String> {
    let mut class = unsafe { ((**env).v1_4.GetObjectClass)(env, target) };
    unsafe { check(env, "resolve instance class")? };
    while !class.is_null() {
        let fields = unsafe {
            call_object(
                env,
                class,
                "getDeclaredFields",
                "()[Ljava/lang/reflect/Field;",
                &[],
            )? as jobjectArray
        };
        let count = unsafe { ((**env).v1_4.GetArrayLength)(env, fields) };
        unsafe { check(env, "read instance fields")? };
        for index in 0..count {
            let field = unsafe { ((**env).v1_4.GetObjectArrayElement)(env, fields, index) };
            unsafe { check(env, "read instance field")? };
            let field_type =
                unsafe { call_object(env, field, "getType", "()Ljava/lang/Class;", &[])? };
            let name =
                unsafe { call_object(env, field_type, "getName", "()Ljava/lang/String;", &[])? };
            if unsafe { java_string(env, name.cast())? } != type_name {
                continue;
            }
            unsafe { call_void(env, field, "setAccessible", "(Z)V", &[bool_value(true)])? };
            return unsafe {
                call_object(
                    env,
                    field,
                    "get",
                    "(Ljava/lang/Object;)Ljava/lang/Object;",
                    &[object_value(target)],
                )
            };
        }
        class =
            unsafe { call_object(env, class, "getSuperclass", "()Ljava/lang/Class;", &[])? }.cast();
    }
    Err(format!("instance field of type {type_name} was not found"))
}

unsafe fn invoke(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    arguments: &[jobject],
) -> Result<jobject, String> {
    if target.is_null() {
        return Err(format!("{name} target is null"));
    }
    let class = unsafe { ((**env).v1_4.GetObjectClass)(env, target) };
    unsafe { check(env, "resolve target class")? };
    let method = unsafe { find_method(env, class, name, arguments.len() as i32)? }
        .ok_or_else(|| format!("method {name}/{} was not found", arguments.len()))?;
    let array = unsafe { object_array(env, arguments)? };
    unsafe {
        call_object(
            env,
            method,
            "invoke",
            "(Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;",
            &[object_value(target), object_value(array)],
        )
    }
}

unsafe fn find_method(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    arity: i32,
) -> Result<Option<jobject>, String> {
    let methods = unsafe {
        call_object(
            env,
            class,
            "getDeclaredMethods",
            "()[Ljava/lang/reflect/Method;",
            &[],
        )? as jobjectArray
    };
    let count = unsafe { ((**env).v1_4.GetArrayLength)(env, methods) };
    unsafe { check(env, "read methods")? };
    for index in 0..count {
        let method = unsafe { ((**env).v1_4.GetObjectArrayElement)(env, methods, index) };
        unsafe { check(env, "read method")? };
        let method_name =
            unsafe { call_object(env, method, "getName", "()Ljava/lang/String;", &[])? };
        if unsafe { java_string(env, method_name.cast())? } != name {
            continue;
        }
        let parameter_count = unsafe { call_int(env, method, "getParameterCount", "()I", &[])? };
        if parameter_count != arity {
            continue;
        }
        unsafe { call_void(env, method, "setAccessible", "(Z)V", &[bool_value(true)])? };
        return Ok(Some(method));
    }
    Ok(None)
}

unsafe fn object_array(env: *mut JNIEnv, values: &[jobject]) -> Result<jobject, String> {
    let class = unsafe { find_class(env, "java/lang/Object")? };
    let array =
        unsafe { ((**env).v1_4.NewObjectArray)(env, values.len() as i32, class, ptr::null_mut()) };
    unsafe { check(env, "create argument array")? };
    for (index, value) in values.iter().enumerate() {
        unsafe { ((**env).v1_4.SetObjectArrayElement)(env, array, index as i32, *value) };
        unsafe { check(env, "fill argument array")? };
    }
    Ok(array.cast())
}

unsafe fn box_long(env: *mut JNIEnv, value: i64) -> Result<jobject, String> {
    let class = unsafe { find_class(env, "java/lang/Long")? };
    unsafe {
        call_static_object(
            env,
            class,
            "valueOf",
            "(J)Ljava/lang/Long;",
            &[long_value(value)],
        )
    }
}

unsafe fn unbox_long(env: *mut JNIEnv, value: jobject) -> Result<i64, String> {
    unsafe { call_long(env, value, "longValue", "()J", &[]) }
}

unsafe fn unbox_number(env: *mut JNIEnv, value: jobject) -> Result<i32, String> {
    unsafe { call_int(env, value, "intValue", "()I", &[]) }
}

unsafe fn object_text(env: *mut JNIEnv, value: jobject) -> Result<String, String> {
    let text = unsafe { call_object(env, value, "toString", "()Ljava/lang/String;", &[])? };
    unsafe { java_string(env, text.cast()) }
}

unsafe fn find_class(env: *mut JNIEnv, name: &str) -> Result<jclass, String> {
    let name = CString::new(name).map_err(|_| "class name contains NUL".to_string())?;
    let class = unsafe { ((**env).v1_4.FindClass)(env, name.as_ptr()) };
    unsafe { check(env, "find class")? };
    if class.is_null() {
        Err("class was not found".to_string())
    } else {
        Ok(class)
    }
}

unsafe fn new_object(
    env: *mut JNIEnv,
    class: jclass,
    signature: &str,
    arguments: &[jvalue],
) -> Result<jobject, String> {
    let name = c"<init>";
    let signature = CString::new(signature).unwrap();
    let method =
        unsafe { ((**env).v1_4.GetMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve constructor")? };
    if method.is_null() {
        return Err("constructor was not found".to_string());
    }
    let value = unsafe { ((**env).v1_4.NewObjectA)(env, class, method, arguments.as_ptr()) };
    unsafe { check(env, "construct object")? };
    if value.is_null() {
        Err("constructor returned null".to_string())
    } else {
        Ok(value)
    }
}

unsafe fn call_object(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<jobject, String> {
    let method = unsafe { method_id(env, target, name, signature)? };
    let value =
        unsafe { ((**env).v1_4.CallObjectMethodA)(env, target, method, arguments.as_ptr()) };
    unsafe { check(env, name)? };
    Ok(value)
}

unsafe fn call_boolean(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<bool, String> {
    let method = unsafe { method_id(env, target, name, signature)? };
    let value =
        unsafe { ((**env).v1_4.CallBooleanMethodA)(env, target, method, arguments.as_ptr()) };
    unsafe { check(env, name)? };
    Ok(value)
}

unsafe fn call_int(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<i32, String> {
    let method = unsafe { method_id(env, target, name, signature)? };
    let value = unsafe { ((**env).v1_4.CallIntMethodA)(env, target, method, arguments.as_ptr()) };
    unsafe { check(env, name)? };
    Ok(value)
}

unsafe fn call_long(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<i64, String> {
    let method = unsafe { method_id(env, target, name, signature)? };
    let value = unsafe { ((**env).v1_4.CallLongMethodA)(env, target, method, arguments.as_ptr()) };
    unsafe { check(env, name)? };
    Ok(value)
}

unsafe fn call_void(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<(), String> {
    let method = unsafe { method_id(env, target, name, signature)? };
    unsafe { ((**env).v1_4.CallVoidMethodA)(env, target, method, arguments.as_ptr()) };
    unsafe { check(env, name) }
}

unsafe fn method_id(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
) -> Result<*mut jni::sys::_jmethodID, String> {
    let class = unsafe { ((**env).v1_4.GetObjectClass)(env, target) };
    unsafe { check(env, "resolve object class")? };
    let name = CString::new(name).unwrap();
    let signature = CString::new(signature).unwrap();
    let method =
        unsafe { ((**env).v1_4.GetMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve method")? };
    if method.is_null() {
        Err("method was not found".to_string())
    } else {
        Ok(method)
    }
}

unsafe fn call_static_object(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<jobject, String> {
    let name = CString::new(name).unwrap();
    let signature = CString::new(signature).unwrap();
    let method =
        unsafe { ((**env).v1_4.GetStaticMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve static method")? };
    if method.is_null() {
        return Err("static method was not found".to_string());
    }
    let value =
        unsafe { ((**env).v1_4.CallStaticObjectMethodA)(env, class, method, arguments.as_ptr()) };
    unsafe { check(env, name.to_string_lossy().as_ref())? };
    Ok(value)
}

unsafe fn call_static_boolean(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<bool, String> {
    let name = CString::new(name).unwrap();
    let signature = CString::new(signature).unwrap();
    let method =
        unsafe { ((**env).v1_4.GetStaticMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve static method")? };
    if method.is_null() {
        return Err("static method was not found".to_string());
    }
    let value =
        unsafe { ((**env).v1_4.CallStaticBooleanMethodA)(env, class, method, arguments.as_ptr()) };
    unsafe { check(env, name.to_string_lossy().as_ref())? };
    Ok(value)
}

unsafe fn call_static_void(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<(), String> {
    let name = CString::new(name).unwrap();
    let signature = CString::new(signature).unwrap();
    let method =
        unsafe { ((**env).v1_4.GetStaticMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve static method")? };
    if method.is_null() {
        return Err("static method was not found".to_string());
    }
    unsafe { ((**env).v1_4.CallStaticVoidMethodA)(env, class, method, arguments.as_ptr()) };
    unsafe { check(env, name.to_string_lossy().as_ref()) }
}

unsafe fn new_string(env: *mut JNIEnv, value: &str) -> Result<jstring, String> {
    let value = CString::new(value).map_err(|_| "string contains NUL".to_string())?;
    let string = unsafe { ((**env).v1_4.NewStringUTF)(env, value.as_ptr()) };
    unsafe { check(env, "create string")? };
    if string.is_null() {
        Err("string allocation failed".to_string())
    } else {
        Ok(string)
    }
}

unsafe fn java_string(env: *mut JNIEnv, value: jstring) -> Result<String, String> {
    if value.is_null() {
        return Ok(String::new());
    }
    let pointer = unsafe { ((**env).v1_4.GetStringUTFChars)(env, value, ptr::null_mut()) };
    unsafe { check(env, "read string")? };
    if pointer.is_null() {
        return Err("string characters are null".to_string());
    }
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    unsafe { ((**env).v1_4.ReleaseStringUTFChars)(env, value, pointer) };
    Ok(text)
}

unsafe fn check(env: *mut JNIEnv, context: &str) -> Result<(), String> {
    if !unsafe { ((**env).v1_4.ExceptionCheck)(env) } {
        return Ok(());
    }
    let exception = unsafe { ((**env).v1_4.ExceptionOccurred)(env) };
    unsafe { ((**env).v1_4.ExceptionClear)(env) };
    if exception.is_null() {
        return Err(format!("{context}: Java exception"));
    }
    let mut detail_source = exception;
    let exception_class = unsafe { ((**env).v1_4.GetObjectClass)(env, exception) };
    let cause_method = unsafe {
        ((**env).v1_4.GetMethodID)(
            env,
            exception_class,
            c"getCause".as_ptr(),
            c"()Ljava/lang/Throwable;".as_ptr(),
        )
    };
    if !cause_method.is_null() {
        let cause =
            unsafe { ((**env).v1_4.CallObjectMethodA)(env, exception, cause_method, ptr::null()) };
        if !cause.is_null() && !unsafe { ((**env).v1_4.ExceptionCheck)(env) } {
            detail_source = cause;
        } else {
            unsafe { ((**env).v1_4.ExceptionClear)(env) };
        }
    }
    let class = unsafe { ((**env).v1_4.GetObjectClass)(env, detail_source) };
    let method = unsafe {
        ((**env).v1_4.GetMethodID)(
            env,
            class,
            c"toString".as_ptr(),
            c"()Ljava/lang/String;".as_ptr(),
        )
    };
    if method.is_null() {
        return Err(format!("{context}: Java exception"));
    }
    let value =
        unsafe { ((**env).v1_4.CallObjectMethodA)(env, detail_source, method, ptr::null()) };
    if value.is_null() || unsafe { ((**env).v1_4.ExceptionCheck)(env) } {
        unsafe { ((**env).v1_4.ExceptionClear)(env) };
        return Err(format!("{context}: Java exception"));
    }
    let detail =
        unsafe { java_string(env, value.cast()) }.unwrap_or_else(|_| "Java exception".to_string());
    Err(format!("{context}: {detail}"))
}

fn native_method(name: &str, signature: &str, function: *mut c_void) -> JNINativeMethod {
    JNINativeMethod {
        name: CString::new(name).unwrap().into_raw(),
        signature: CString::new(signature).unwrap().into_raw(),
        fnPtr: function,
    }
}

fn object_value(value: jobject) -> jvalue {
    jvalue { l: value }
}

fn long_value(value: i64) -> jvalue {
    jvalue { j: value }
}

pub(crate) fn int_value(value: i32) -> jvalue {
    jvalue { i: value }
}

fn bool_value(value: bool) -> jvalue {
    jvalue { z: value }
}

fn write_response(stream: &mut TcpStream, id: u64, result: CommandResult) -> Result<(), String> {
    match result {
        Ok(value) => write_json(
            stream,
            &Response {
                id,
                ok: true,
                error: None,
                value: value.as_deref(),
            },
        ),
        Err(error) => write_json(
            stream,
            &Response {
                id,
                ok: false,
                error: Some(&error),
                value: None,
            },
        ),
    }
}

fn write_json(stream: &mut TcpStream, value: &impl Serialize) -> Result<(), String> {
    serde_json::to_writer(&mut *stream, value).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())
}

fn load_lsplant() -> Result<*mut c_void, String> {
    let name = CString::new("noa-kakao-lsplant").unwrap();
    let fd =
        unsafe { libc::syscall(libc::SYS_memfd_create, name.as_ptr(), libc::MFD_CLOEXEC) } as c_int;
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(LSPLANT).map_err(|error| error.to_string())?;
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

fn log(priority: c_int, message: &str) {
    let Ok(message) = CString::new(message) else {
        return;
    };
    unsafe {
        __android_log_write(priority, c"NoaKakaoAgent".as_ptr(), message.as_ptr());
    }
}
