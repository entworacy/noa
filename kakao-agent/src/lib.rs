use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_int, c_void},
    fs::File,
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    os::fd::FromRawFd,
    ptr,
    sync::{Arc, Condvar, Mutex, Once, OnceLock, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use jni::sys::{
    JNI_EDETACHED, JNI_OK, JNI_VERSION_1_6, JNIEnv, JNINativeMethod, JavaVM, jboolean, jclass,
    jint, jlong, jobject, jobjectArray, jstring, jvalue,
};
use serde::{Deserialize, Serialize};

const ADAPTER_DEX: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../assets/noa-kakao-agent.dex"
));
const ACTION_SEND: i32 = 1;
const ACTION_KICK: i32 = 2;
const ACTION_CHATONROOM: i32 = 3;
const KIND_LOCO_SEND: i32 = 1;
const KIND_LOCO_RECEIVE: i32 = 2;
const LOG_INFO: c_int = 4;
const LOG_ERROR: c_int = 6;

static START: Once = Once::new();
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static COMMANDS: OnceLock<Mutex<HashMap<u64, Arc<CommandState>>>> = OnceLock::new();
static EVENTS: OnceLock<mpsc::SyncSender<String>> = OnceLock::new();

const LSPLANT: &[u8] = include_bytes!(env!("NOA_LSPLANT_BLOB"));

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlerror() -> *const c_char;
    fn noa_lsplant_init(env: *mut JNIEnv, handle: *mut c_void) -> bool;
    fn noa_lsplant_hook(
        env: *mut JNIEnv,
        handle: *mut c_void,
        target: jobject,
        hooker: jobject,
        callback: jobject,
    ) -> jobject;
    fn noa_lsplant_deoptimize(
        env: *mut JNIEnv,
        handle: *mut c_void,
        target: jobject,
    ) -> bool;
}

type GetCreatedVms = unsafe extern "system" fn(*mut *mut JavaVM, i32, *mut i32) -> i32;

#[derive(Deserialize)]
struct Bootstrap {
    port: u16,
    event_port: u16,
    token: String,
}

#[derive(Deserialize)]
struct Request {
    token: String,
    id: u64,
    action: String,
    room: Option<i64>,
    row: Option<i64>,
    user: Option<i64>,
}

#[derive(Serialize)]
struct Response<'a> {
    id: u64,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

#[derive(Serialize)]
struct Hello<'a> {
    event: &'static str,
    token: &'a str,
    pid: u32,
    protocol: u8,
}

#[derive(Clone)]
enum Operation {
    Send { room: i64, row: i64 },
    Kick { room: i64, user: i64 },
    ChatOnRoom { room: i64 },
}

#[derive(Default)]
struct Progress {
    loaded: bool,
    result: Option<Result<(), String>>,
}

struct CommandState {
    operation: Operation,
    progress: Mutex<Progress>,
    changed: Condvar,
}

struct Runtime {
    vm: usize,
    loader: usize,
    lsplant: usize,
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
    let event_address = SocketAddr::from(([127, 0, 0, 1], config.event_port));
    let (event_sender, event_receiver) = mpsc::sync_channel(1024);
    let _ = EVENTS.set(event_sender);
    let event_token = config.token.clone();
    let _ = thread::Builder::new()
        .name("noa-kakao-events".to_string())
        .spawn(move || event_loop(event_address, event_token, event_receiver));
    loop {
        match TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
            Ok(mut stream) => {
                if let Err(error) = session(&mut stream, &config.token) {
                    log(LOG_ERROR, &error);
                }
            }
            Err(_) => thread::sleep(Duration::from_secs(1)),
        }
    }
}

fn event_loop(address: SocketAddr, token: String, receiver: mpsc::Receiver<String>) {
    let mut stream = None;
    while let Ok(payload) = receiver.recv() {
        let message = serde_json::json!({"token": token, "event": "loco", "packet": payload});
        let mut value: serde_json::Value = serde_json::from_str(&payload).unwrap_or_default();
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "token".to_string(),
                serde_json::Value::String(token.clone()),
            );
            object.insert(
                "event".to_string(),
                serde_json::Value::String("loco".to_string()),
            );
        } else {
            value = message;
        }
        let line = value.to_string() + "\n";
        loop {
            if stream.is_none() {
                stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).ok();
            }
            let Some(active) = stream.as_mut() else {
                thread::sleep(Duration::from_millis(500));
                continue;
            };
            if active
                .write_all(line.as_bytes())
                .and_then(|_| active.flush())
                .is_ok()
            {
                break;
            }
            stream = None;
        }
    }
}

fn session(stream: &mut TcpStream, token: &str) -> Result<(), String> {
    initialize_runtime()?;
    stream
        .set_nodelay(true)
        .map_err(|error| error.to_string())?;
    let hello = Hello {
        event: "ready",
        token,
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
        let request = match serde_json::from_str::<Request>(line.trim()) {
            Ok(request) => request,
            Err(error) => {
                write_response(stream, 0, Err(format!("invalid command: {error}")))?;
                continue;
            }
        };
        if request.token != token {
            write_response(stream, request.id, Err("authentication failed".to_string()))?;
            continue;
        }
        let result = execute(request);
        write_response(stream, result.0, result.1)?;
    }
}

fn execute(request: Request) -> (u64, Result<(), String>) {
    let id = request.id;
    let operation = match request.action.as_str() {
        "send-custom" => match (request.room, request.row) {
            (Some(room), Some(row)) => Operation::Send { room, row },
            _ => return (id, Err("room and row are required".to_string())),
        },
        "kick-member" => match (request.room, request.user) {
            (Some(room), Some(user)) => Operation::Kick { room, user },
            _ => return (id, Err("room and user are required".to_string())),
        },
        "chat-on-room" => match request.room {
            Some(room) => Operation::ChatOnRoom { room },
            None => return (id, Err("room is required".to_string())),
        },
        _ => return (id, Err("unsupported action".to_string())),
    };
    let state = Arc::new(CommandState {
        operation: operation.clone(),
        progress: Mutex::new(Progress::default()),
        changed: Condvar::new(),
    });
    commands().lock().unwrap().insert(id, state.clone());
    let result = match operation {
        Operation::Send { .. } => execute_send(id, &state),
        Operation::Kick { .. } => execute_kick(id, &state),
        Operation::ChatOnRoom { .. } => execute_chat_on_room(id, &state),
    };
    commands().lock().unwrap().remove(&id);
    (id, result)
}

fn execute_send(id: u64, state: &CommandState) -> Result<(), String> {
    with_env(|env| unsafe { start_sending_log_load(env, id) })?;
    wait_loaded(state, Duration::from_secs(10))?;
    with_env(|env| unsafe { post_main(env, id, ACTION_SEND) })?;
    wait_result(state, Duration::from_secs(10))
}

fn execute_kick(id: u64, state: &CommandState) -> Result<(), String> {
    with_env(|env| unsafe { post_main(env, id, ACTION_KICK) })?;
    wait_result(state, Duration::from_secs(10))
}

fn execute_chat_on_room(id: u64, state: &CommandState) -> Result<(), String> {
    with_env(|env| unsafe { post_main(env, id, ACTION_CHATONROOM) })?;
    wait_result(state, Duration::from_secs(10))
}

fn wait_loaded(state: &CommandState, timeout: Duration) -> Result<(), String> {
    let progress = state.progress.lock().unwrap();
    let (progress, result) = state
        .changed
        .wait_timeout_while(progress, timeout, |value| {
            !value.loaded && value.result.is_none()
        })
        .unwrap();
    if let Some(result) = progress.result.clone() {
        return result;
    }
    if result.timed_out() && !progress.loaded {
        return Err("sending log load timed out".to_string());
    }
    Ok(())
}

fn wait_result(state: &CommandState, timeout: Duration) -> Result<(), String> {
    let progress = state.progress.lock().unwrap();
    let (progress, result) = state
        .changed
        .wait_timeout_while(progress, timeout, |value| value.result.is_none())
        .unwrap();
    if let Some(value) = progress.result.clone() {
        return value;
    }
    if result.timed_out() {
        Err("KakaoTalk native call timed out".to_string())
    } else {
        Err("KakaoTalk native call ended without a result".to_string())
    }
}

fn mark_loaded(id: u64) {
    if let Some(state) = commands().lock().unwrap().get(&id).cloned() {
        let mut progress = state.progress.lock().unwrap();
        progress.loaded = true;
        state.changed.notify_all();
    }
}

fn mark_complete(id: u64, result: Result<(), String>) {
    if let Some(state) = commands().lock().unwrap().get(&id).cloned() {
        let mut progress = state.progress.lock().unwrap();
        progress.result = Some(result);
        state.changed.notify_all();
    }
}

fn initialize_runtime() -> Result<(), String> {
    if RUNTIME.get().is_some() {
        return Ok(());
    }
    let vm = unsafe { locate_vm() }?;
    let loader = with_attached(vm, |env| unsafe { create_loader(env) })?;
    let lsplant = load_lsplant()?;
    let runtime = Runtime {
        vm: vm as usize,
        loader: loader as usize,
        lsplant: lsplant as usize,
    };
    let _ = RUNTIME.set(runtime);
    with_attached(vm, |env| unsafe {
        if !noa_lsplant_init(env, lsplant) {
            return Err("LSPlant initialization failed".to_string());
        }
        install_hook(env, "gt.h", "y", &["mt.k"], KIND_LOCO_SEND, false, 1)?;
        install_hook(
            env,
            "gt.h$b",
            "b",
            &["mt.l", "kotlin.coroutines.Continuation"],
            KIND_LOCO_RECEIVE,
            false,
            1,
        )?;
        deoptimize_method(env, "gt.h$d", "invokeSuspend", &["java.lang.Object"])?;
        Ok(())
    })?;
    log(LOG_INFO, "Rust KakaoTalk agent ready");
    Ok(())
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
    let application = unsafe {
        call_static_object(
            env,
            activity_thread,
            "currentApplication",
            "()Landroid/app/Application;",
            &[],
        )?
    };
    if application.is_null() {
        return Err("Android application is not ready".to_string());
    }
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
    let bridge = unsafe { load_class(env, global, "dev.noa.kakao.Bridge")? };
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
    ];
    let status = unsafe { ((**env).v1_4.RegisterNatives)(env, bridge, methods.as_ptr(), 4) };
    unsafe { check(env, "register native callbacks")? };
    if status != JNI_OK {
        return Err(format!("RegisterNatives failed: {status}"));
    }
    for name in [
        "dev.noa.kakao.LoadContinuation",
        "dev.noa.kakao.MainDispatch",
        "dev.noa.kakao.SendListener",
        "dev.noa.kakao.KickListener",
        "dev.noa.kakao.Hooker",
    ] {
        unsafe { load_class(env, global, name)? };
    }
    Ok(global)
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
        ACTION_SEND => unsafe { dispatch_send(env, id as u64) },
        ACTION_KICK => unsafe { dispatch_kick(env, id as u64) },
        ACTION_CHATONROOM => unsafe { dispatch_chat_on_room(env, id as u64) },
        _ => Err("unknown main-thread action".to_string()),
    };
    if let Err(error) = result {
        mark_complete(id as u64, Err(error));
    }
}

unsafe extern "system" fn bridge_capture(env: *mut JNIEnv, _: jclass, kind: jint, packet: jobject) {
    if packet.is_null() {
        return;
    }
    let result = (|| unsafe {
        let header = instance_field_by_type(env, packet, "mt.b")?;
        let body = instance_field_by_type(env, packet, "mt.a")?;
        let packet_id = unbox_number(env, instance_field(env, header, "a")?)?;
        let status = unbox_number(env, instance_field(env, header, "b")?)?;
        let method = object_text(env, instance_field(env, header, "c")?)?;
        let body_length = unbox_number(env, instance_field(env, header, "d")?)?;
        let bson = instance_field(env, body, "a")?;
        let body = object_text(env, bson)?;
        let direction = if kind == KIND_LOCO_SEND {
            "send"
        } else {
            "receive"
        };
        let payload = serde_json::json!({
            "direction": direction,
            "method": method,
            "packetId": packet_id,
            "status": status,
            "bodyLength": body_length,
            "body": body,
            "capturedAt": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64,
        });
        if let Some(sender) = EVENTS.get() {
            let _ = sender.try_send(payload.to_string());
        }
        Ok::<(), String>(())
    })();
    if let Err(error) = result {
        log(LOG_ERROR, &format!("LOCO capture failed: {error}"));
    }
}

unsafe fn start_sending_log_load(env: *mut JNIEnv, id: u64) -> Result<(), String> {
    let manager_class = unsafe { app_class(env, "com.kakao.talk.manager.send.sending.b")? };
    let manager = unsafe { static_field(env, manager_class, "a")? };
    let continuation_class = unsafe { app_class(env, "dev.noa.kakao.LoadContinuation")? };
    let context_class = unsafe { app_class(env, "kotlin.coroutines.EmptyCoroutineContext")? };
    let context = unsafe { static_field(env, context_class, "C")? };
    let continuation = unsafe {
        new_object(
            env,
            continuation_class,
            "(JLkotlin/coroutines/CoroutineContext;)V",
            &[long_value(id as i64), object_value(context)],
        )?
    };
    let result = unsafe { invoke(env, manager, "G", &[continuation])? };
    if result.is_null() || unsafe { object_text(env, result) }? != "COROUTINE_SUSPENDED" {
        mark_loaded(id);
    }
    Ok(())
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

unsafe fn dispatch_send(env: *mut JNIEnv, id: u64) -> Result<(), String> {
    let operation = command_operation(id)?;
    let Operation::Send { room, row } = operation else {
        return Err("send command state mismatch".to_string());
    };
    let manager_class = unsafe { app_class(env, "com.kakao.talk.manager.send.sending.b")? };
    let manager = unsafe { static_field(env, manager_class, "a")? };
    let map = unsafe { static_field(env, manager_class, "b")? };
    let key = unsafe { box_long(env, room)? };
    let queue = unsafe {
        call_object(
            env,
            map,
            "get",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            &[object_value(key)],
        )?
    };
    if queue.is_null() {
        return Err(format!("sending log room not loaded: {room}"));
    }
    let iterator = unsafe { call_object(env, queue, "iterator", "()Ljava/util/Iterator;", &[])? };
    let mut entry = ptr::null_mut();
    while unsafe { call_boolean(env, iterator, "hasNext", "()Z", &[])? } {
        let candidate = unsafe { call_object(env, iterator, "next", "()Ljava/lang/Object;", &[])? };
        let value = unsafe { call_long(env, candidate, "getId", "()J", &[])? };
        if value == row {
            entry = candidate;
            break;
        }
    }
    if entry.is_null() {
        return Err(format!("sending log row not loaded: {row}"));
    }
    let room_object = unsafe { find_room(env, room)? };
    unsafe { invoke(env, manager, "P", &[entry])? };
    let request_class =
        unsafe { app_class(env, "com.kakao.talk.manager.send.ChatSendingLogRequest")? };
    let companion = unsafe { static_object_with_method(env, request_class, "u", 5)? };
    let mode_class =
        unsafe { app_class(env, "com.kakao.talk.manager.send.ChatSendingLogRequest$d")? };
    let mode_name = unsafe { new_string(env, "Resend")? };
    let enum_class = unsafe { find_class(env, "java/lang/Enum")? };
    let mode = unsafe {
        call_static_object(
            env,
            enum_class,
            "valueOf",
            "(Ljava/lang/Class;Ljava/lang/String;)Ljava/lang/Enum;",
            &[object_value(mode_class), object_value(mode_name)],
        )?
    };
    let listener_class = unsafe { app_class(env, "dev.noa.kakao.SendListener")? };
    let listener = unsafe { new_object(env, listener_class, "(J)V", &[long_value(id as i64)])? };
    let no = unsafe { box_boolean(env, false)? };
    unsafe {
        invoke(
            env,
            companion,
            "u",
            &[room_object, entry, mode, listener, no],
        )?
    };
    Ok(())
}

unsafe fn dispatch_kick(env: *mut JNIEnv, id: u64) -> Result<(), String> {
    let operation = command_operation(id)?;
    let Operation::Kick { room, user } = operation else {
        return Err("kick command state mismatch".to_string());
    };
    let room_object = unsafe { find_room(env, room)? };
    let user_value = unsafe { box_long(env, user)? };
    let contains = unsafe { invoke(env, room_object, "N1", &[user_value])? };
    if !unsafe { unbox_boolean(env, contains)? } {
        return Err(format!("user is not a current chat member: {user}"));
    }
    let link = unsafe { invoke(env, room_object, "J0", &[])? };
    let link_id = unsafe { unbox_long(env, link)? };
    let members_class = unsafe { app_class(env, "t50.I0")? };
    let members = unsafe { static_field(env, members_class, "a")? };
    let link_value = unsafe { box_long(env, link_id)? };
    let member = unsafe { invoke(env, members, "o", &[user_value, link_value])? };
    if member.is_null() {
        return Err(format!("open chat member not found: {user}"));
    }
    let manager_class = unsafe { app_class(env, "UV.h")? };
    let manager_root = unsafe { static_field(env, manager_class, "C")? };
    let foreground = unsafe { invoke(env, manager_root, "p", &[])? };
    let listener_class = unsafe { app_class(env, "dev.noa.kakao.KickListener")? };
    let listener = unsafe { new_object(env, listener_class, "(J)V", &[long_value(id as i64)])? };
    let no = unsafe { box_boolean(env, false)? };
    unsafe {
        invoke(
            env,
            foreground,
            "f",
            &[room_object, member, no, no, listener],
        )?
    };
    Ok(())
}

unsafe fn dispatch_chat_on_room(env: *mut JNIEnv, id: u64) -> Result<(), String> {
    let Operation::ChatOnRoom { room } = command_operation(id)? else {
        return Err("CHATONROOM command state mismatch".to_string());
    };
    if !unsafe { loco_connected(env)? } {
        mark_complete(id, Ok(()));
        return Ok(());
    }
    let room_object = unsafe { find_room(env, room)? };
    let helper_class = unsafe { app_class(env, "Yr.l0")? };
    let helper = unsafe { static_object_with_method(env, helper_class, "h1", 2)? };
    let continuation_class = unsafe { app_class(env, "dev.noa.kakao.LoadContinuation")? };
    let context_class = unsafe { app_class(env, "kotlin.coroutines.EmptyCoroutineContext")? };
    let context = unsafe { static_field(env, context_class, "C")? };
    let continuation = unsafe {
        new_object(
            env,
            continuation_class,
            "(JLkotlin/coroutines/CoroutineContext;)V",
            &[long_value(id as i64), object_value(context)],
        )?
    };
    unsafe { invoke(env, helper, "h1", &[room_object, continuation])? };
    mark_complete(id, Ok(()));
    Ok(())
}

unsafe fn loco_connected(env: *mut JNIEnv) -> Result<bool, String> {
    let core_class = unsafe { app_class(env, "Us.d")? };
    let core = unsafe { static_field(env, core_class, "b")? };
    let flow = unsafe { invoke(env, core, "S", &[])? };
    let state = unsafe { call_object(env, flow, "getValue", "()Ljava/lang/Object;", &[])? };
    Ok(unsafe { object_text(env, state)? }.eq_ignore_ascii_case("connected"))
}

unsafe fn find_room(env: *mut JNIEnv, room: i64) -> Result<jobject, String> {
    let roots = unsafe { app_class(env, "Yr.c1")? };
    let holder = unsafe { static_field(env, roots, "n")? };
    let repository = unsafe { invoke(env, holder, "j", &[])? };
    let room_value = unsafe { box_long(env, room)? };
    let result = unsafe { invoke(env, repository, "d0", &[room_value])? };
    if result.is_null() {
        Err(format!("chat room not found: {room}"))
    } else {
        Ok(result)
    }
}

fn command_operation(id: u64) -> Result<Operation, String> {
    commands()
        .lock()
        .unwrap()
        .get(&id)
        .map(|state| state.operation.clone())
        .ok_or_else(|| "command state was removed".to_string())
}

fn commands() -> &'static Mutex<HashMap<u64, Arc<CommandState>>> {
    COMMANDS.get_or_init(|| Mutex::new(HashMap::new()))
}

unsafe fn install_hook(
    env: *mut JNIEnv,
    class_name: &str,
    method_name: &str,
    parameter_types: &[&str],
    kind: i32,
    static_target: bool,
    packet_index: i32,
) -> Result<(), String> {
    let target_class = unsafe { app_class(env, class_name)? };
    let target = unsafe { find_exact_method(env, target_class, method_name, parameter_types)? };
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
        return Err(format!(
            "LSPlant returned no backup for {class_name}.{method_name}"
        ));
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

unsafe fn deoptimize_method(
    env: *mut JNIEnv,
    class_name: &str,
    method_name: &str,
    parameter_types: &[&str],
) -> Result<(), String> {
    let target_class = unsafe { app_class(env, class_name)? };
    let target = unsafe { find_exact_method(env, target_class, method_name, parameter_types)? };
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "runtime is unavailable".to_string())?;
    if unsafe {
        noa_lsplant_deoptimize(env, runtime.lsplant as *mut c_void, target)
    } {
        Ok(())
    } else {
        Err(format!(
            "LSPlant could not deoptimize {class_name}.{method_name}"
        ))
    }
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

unsafe fn static_object_with_method(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    arity: i32,
) -> Result<jobject, String> {
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
    unsafe { check(env, "read static fields")? };
    for index in 0..count {
        let field = unsafe { ((**env).v1_4.GetObjectArrayElement)(env, fields, index) };
        unsafe { check(env, "read static field")? };
        let modifiers = unsafe { call_int(env, field, "getModifiers", "()I", &[])? };
        let modifier = unsafe { find_class(env, "java/lang/reflect/Modifier")? };
        let is_static = unsafe {
            call_static_boolean(env, modifier, "isStatic", "(I)Z", &[int_value(modifiers)])?
        };
        if !is_static {
            continue;
        }
        unsafe { call_void(env, field, "setAccessible", "(Z)V", &[bool_value(true)])? };
        let value = unsafe {
            call_object(
                env,
                field,
                "get",
                "(Ljava/lang/Object;)Ljava/lang/Object;",
                &[object_value(ptr::null_mut())],
            )?
        };
        if value.is_null() {
            continue;
        }
        let value_class = unsafe { ((**env).v1_4.GetObjectClass)(env, value) };
        if unsafe { find_method(env, value_class, name, arity)? }.is_some() {
            return Ok(value);
        }
    }
    Err(format!(
        "static companion with {name}/{arity} was not found"
    ))
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

unsafe fn box_boolean(env: *mut JNIEnv, value: bool) -> Result<jobject, String> {
    let class = unsafe { find_class(env, "java/lang/Boolean")? };
    unsafe {
        call_static_object(
            env,
            class,
            "valueOf",
            "(Z)Ljava/lang/Boolean;",
            &[bool_value(value)],
        )
    }
}

unsafe fn unbox_long(env: *mut JNIEnv, value: jobject) -> Result<i64, String> {
    unsafe { call_long(env, value, "longValue", "()J", &[]) }
}

unsafe fn unbox_number(env: *mut JNIEnv, value: jobject) -> Result<i32, String> {
    unsafe { call_int(env, value, "intValue", "()I", &[]) }
}

unsafe fn unbox_boolean(env: *mut JNIEnv, value: jobject) -> Result<bool, String> {
    unsafe { call_boolean(env, value, "booleanValue", "()Z", &[]) }
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

fn int_value(value: i32) -> jvalue {
    jvalue { i: value }
}

fn bool_value(value: bool) -> jvalue {
    jvalue { z: value }
}

fn write_response(
    stream: &mut TcpStream,
    id: u64,
    result: Result<(), String>,
) -> Result<(), String> {
    match result {
        Ok(()) => write_json(
            stream,
            &Response {
                id,
                ok: true,
                error: None,
            },
        ),
        Err(error) => write_json(
            stream,
            &Response {
                id,
                ok: false,
                error: Some(&error),
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
    let path = CString::new(format!("/proc/self/fd/{fd}")).unwrap();
    let handle = unsafe { dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
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
