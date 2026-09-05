mod transport;
use transport::serve;
mod java;
pub(crate) use java::{
    bool_value, box_long, call_boolean, call_long, call_object, call_static_boolean,
    call_static_object, call_static_void, check, find_class, find_exact_method, instance_field,
    instance_field_by_type, int_value, invoke, java_string, load_class, long_value, native_method,
    new_object, new_string, object_array, object_text, object_value, static_field, unbox_long,
    unbox_number,
};
use noa_agent_runtime::{
    jvm::{locate_vm, with_attached},
    lsplant::{
        initialization_error as lsplant_initialization_error, noa_lsplant_deoptimize,
        noa_lsplant_hook, noa_lsplant_init, noa_lsplant_uses_shorty_fallback,
    },
};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    io::Write,
    net::{SocketAddr, TcpStream},
    ptr,
    sync::{Once, OnceLock},
    thread,
    time::Duration,
};

use jni::sys::{
    JNI_OK, JNIEnv, JavaVM, jboolean, jclass, jint, jlong, jobject, jobjectArray, jstring,
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
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

#[derive(Clone, Deserialize)]
struct Bootstrap {
    port: u16,
    event_port: u16,
    vox_port: u16,
    audio_port: u16,
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
    channel: &'a str,
}

#[derive(Serialize)]
struct FailureHello<'a> {
    event: &'static str,
    token: &'a str,
    pid: u32,
    protocol: u8,
    channel: &'a str,
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
        .get_or_init(|| unsafe {
            with_attached(runtime.vm as *mut JavaVM, |env| {
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

fn bootstrap_runtime() -> Result<&'static Runtime, String> {
    if let Some(runtime) = RUNTIME.get() {
        return Ok(runtime);
    }
    log(LOG_INFO, "Kakao agent initialization: locating JVM");
    let vm = unsafe { locate_vm() }.map_err(|error| format!("locate JVM: {error}"))?;
    log(LOG_INFO, "Kakao agent initialization: loading LSPlant");
    let lsplant = load_lsplant().map_err(|error| format!("load LSPlant: {error}"))?;
    log(
        LOG_INFO,
        "Kakao agent initialization: waiting for Android application",
    );
    let loader = unsafe { with_attached(vm, |env| create_loader(env)) }
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
    unsafe { with_attached(runtime.vm as *mut JavaVM, run) }
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

unsafe fn app_class(env: *mut JNIEnv, name: &str) -> Result<jclass, String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "native runtime is not initialized".to_string())?;
    unsafe { load_class(env, runtime.loader as jobject, name) }
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
    noa_agent_runtime::lsplant::load(LSPLANT, c"noa-kakao-lsplant")
}

fn log(priority: c_int, message: &str) {
    let Ok(message) = CString::new(message) else {
        return;
    };
    unsafe {
        __android_log_write(priority, c"NoaKakaoAgent".as_ptr(), message.as_ptr());
    }
}
