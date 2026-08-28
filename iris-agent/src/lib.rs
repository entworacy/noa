use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString, c_char, c_int, c_void},
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    os::fd::FromRawFd,
    ptr,
    sync::{
        Mutex, Once, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use jni::sys::{
    JNI_EDETACHED, JNI_OK, JNI_VERSION_1_6, JNIEnv, JNINativeMethod, JavaVM, jclass, jint, jlong,
    jobject, jobjectArray, jstring, jvalue,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const ADAPTER_DEX: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../assets/noa-iris-agent.dex"
));
const LSPLANT: &[u8] = include_bytes!(env!("NOA_LSPLANT_BLOB"));
const KIND_DESERIALIZE: i32 = 1;
const KIND_SEND: i32 = 2;
const KIND_ROUTING: i32 = 3;
const LOG_INFO: c_int = 4;
const LOG_ERROR: c_int = 6;

static START: Once = Once::new();
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static POLICY: OnceLock<Policy> = OnceLock::new();
static PENDING: OnceLock<Mutex<HashMap<String, PendingReply>>> = OnceLock::new();
static SEQUENCE: Mutex<u64> = Mutex::new(0);
static ROUTING_INTERCEPTED: AtomicBool = AtomicBool::new(false);
static ROUTING_INSPECTION_BYPASSED: AtomicBool = AtomicBool::new(false);
static ENDPOINT_DISPATCHED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
    fn noa_dlopen_fd(fd: c_int, flags: c_int) -> *mut c_void;
    fn noa_lsplant_init(env: *mut JNIEnv, handle: *mut c_void) -> bool;
    fn noa_lsplant_hook(
        env: *mut JNIEnv,
        handle: *mut c_void,
        target: jobject,
        hooker: jobject,
        callback: jobject,
    ) -> jobject;
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

type GetCreatedVms = unsafe extern "system" fn(*mut *mut JavaVM, i32, *mut i32) -> i32;

#[derive(Deserialize)]
struct Bootstrap {
    port: u16,
    token: String,
    types: Vec<String>,
    endpoint_prefix: String,
}

struct Policy {
    address: SocketAddr,
    token: String,
    types: HashSet<String>,
    endpoint_prefix: String,
    marker: String,
}

struct Runtime {
    loader: usize,
    lsplant: usize,
}

unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

struct PendingReply {
    payload: String,
    reply_type: String,
    room: String,
    created: Instant,
}

#[derive(Serialize)]
struct BridgeRequest<'a> {
    event: &'static str,
    token: &'a str,
    pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<&'a str>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    reply_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    room: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uri: Option<&'a str>,
    #[serde(rename = "contentType", skip_serializing_if = "Option::is_none")]
    content_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
}

struct EndpointResponse {
    status: i32,
    content_type: String,
    body: String,
}

#[unsafe(no_mangle)]
#[allow(clippy::missing_safety_doc)]
pub unsafe extern "C" fn noa_iris_agent_main(data: *const c_char, stay_resident: *mut c_int) {
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
        log(LOG_ERROR, "invalid Iris bootstrap payload");
        return;
    };
    START.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("noa-iris-agent".to_string())
            .spawn(move || start(config));
    });
}

fn start(config: Bootstrap) {
    let types = config
        .types
        .into_iter()
        .filter(|value| matches!(value.as_str(), "file" | "markdown" | "custom"))
        .collect::<HashSet<_>>();
    if types.is_empty() {
        log(LOG_ERROR, "Iris interception policy is empty");
        return;
    }
    let marker = format!(
        "__NOA_NATIVE_{}_{}_",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let _ = POLICY.set(Policy {
        address: SocketAddr::from(([127, 0, 0, 1], config.port)),
        token: config.token,
        types,
        endpoint_prefix: config.endpoint_prefix,
        marker,
    });
    match initialize_runtime() {
        Ok(()) => {
            log(LOG_INFO, "Rust Iris hooks installed");
            if let Err(error) = notify_ready() {
                log(
                    LOG_ERROR,
                    &format!("Iris ready notification failed: {error}"),
                );
            }
        }
        Err(error) => log(
            LOG_ERROR,
            &format!("Iris native hook initialization failed: {error}"),
        ),
    }
}

fn initialize_runtime() -> Result<(), String> {
    let vm = unsafe { locate_vm() }?;
    with_attached(vm, |env| unsafe {
        let parent = system_loader(env)?;
        let loader = create_loader(env, parent)?;
        register_bridge(env, loader)?;
        let lsplant = load_lsplant()?;
        if !noa_lsplant_init(env, lsplant) {
            return Err("LSPlant initialization failed".to_string());
        }
        let _ = RUNTIME.set(Runtime {
            loader: loader as usize,
            lsplant: lsplant as usize,
        });
        install_hook(
            env,
            parent,
            "party.qwer.iris.model.ReplyRequest$$serializer",
            "deserialize",
            &["kotlinx.serialization.encoding.Decoder"],
            KIND_DESERIALIZE,
        )?;
        install_hook(
            env,
            parent,
            "party.qwer.iris.Replier$Companion",
            "sendMessage",
            &[
                "java.lang.String",
                "long",
                "java.lang.String",
                "java.lang.Long",
            ],
            KIND_SEND,
        )?;
        install_hook(
            env,
            parent,
            "io.ktor.server.routing.RoutingRoot",
            "interceptor",
            &[
                "io.ktor.util.pipeline.PipelineContext",
                "kotlin.coroutines.Continuation",
            ],
            KIND_ROUTING,
        )?;
        Ok(())
    })
}

unsafe fn install_hook(
    env: *mut JNIEnv,
    parent: jobject,
    class_name: &str,
    method_name: &str,
    parameter_types: &[&str],
    kind: i32,
) -> Result<(), String> {
    let target_class = unsafe { load_class(env, parent, class_name)? };
    let target = unsafe { find_method(env, target_class, method_name, parameter_types)? };
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "runtime is unavailable".to_string())?;
    let hooker_class =
        unsafe { load_class(env, runtime.loader as jobject, "dev.noa.iris.Hooker")? };
    let hooker = unsafe { new_object(env, hooker_class, "(I)V", &[int_value(kind)])? };
    let callback = unsafe { find_method(env, hooker_class, "callback", &["[Ljava.lang.Object;"])? };
    let backup = unsafe {
        noa_lsplant_hook(
            env,
            runtime.lsplant as *mut c_void,
            target,
            hooker,
            callback,
        )
    };
    unsafe { check(env, "install LSPlant hook")? };
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
    unsafe { check(env, "store hook backup")? };
    let retained = unsafe { ((**env).v1_4.NewGlobalRef)(env, hooker) };
    unsafe { check(env, "retain hooker")? };
    if retained.is_null() {
        return Err("hooker global reference is null".to_string());
    }
    Ok(())
}

unsafe extern "system" fn bridge_invoke(
    env: *mut JNIEnv,
    _: jclass,
    kind: jint,
    backup: jobject,
    args: jobjectArray,
) -> jobject {
    let result = match kind {
        KIND_DESERIALIZE => unsafe { intercept_deserialize(env, args) },
        KIND_SEND => unsafe { intercept_send(env, backup, args) },
        KIND_ROUTING => unsafe { intercept_routing(env, backup, args) },
        _ => Err("unknown Iris hook callback".to_string()),
    };
    match result {
        Ok(value) => value,
        Err(message) => {
            unsafe { throw_runtime(env, &message) };
            ptr::null_mut()
        }
    }
}

unsafe extern "system" fn bridge_endpoint(
    env: *mut JNIEnv,
    _: jclass,
    method: jstring,
    uri: jstring,
    content_type: jstring,
    body: jstring,
) -> jobject {
    let result = (|| {
        let method = unsafe { java_string(env, method)? };
        let uri = unsafe { java_string(env, uri)? };
        let content_type = unsafe { java_string(env, content_type)? };
        let body = unsafe { java_string(env, body)? };
        let response = forward_endpoint(&method, &uri, &content_type, &body)?;
        unsafe { new_endpoint_response(env, response) }
    })();
    match result {
        Ok(value) => value,
        Err(message) => {
            unsafe { throw_runtime(env, &message) };
            ptr::null_mut()
        }
    }
}

unsafe fn intercept_deserialize(env: *mut JNIEnv, args: jobjectArray) -> Result<jobject, String> {
    if unsafe { array_length(env, args)? } != 2 {
        return Err("unexpected ReplyRequest.deserialize arguments".to_string());
    }
    let decoder = unsafe { array_element(env, args, 1)? };
    let serializer_class =
        unsafe { app_class(env, "kotlinx.serialization.json.JsonElementSerializer")? };
    let serializer = unsafe {
        static_object(
            env,
            serializer_class,
            "INSTANCE",
            "Lkotlinx/serialization/json/JsonElementSerializer;",
        )?
    };
    let serializer_method = unsafe {
        find_method(
            env,
            serializer_class,
            "deserialize",
            &["kotlinx.serialization.encoding.Decoder"],
        )?
    };
    let decoded = unsafe { invoke_reflect(env, serializer_method, serializer, &[decoder])? };
    let raw = unsafe { object_text(env, decoded)? };
    let payload: Value =
        serde_json::from_str(&raw).map_err(|error| format!("invalid Iris /reply JSON: {error}"))?;
    let object = payload
        .as_object()
        .ok_or_else(|| "Iris /reply body must be a JSON object".to_string())?;
    let reply_type = match object.get("type") {
        None => "text",
        Some(Value::String(value)) => value.as_str(),
        Some(_) => return Err("Iris /reply type must be a string".to_string()),
    };
    let policy = policy()?;
    let selected = policy.types.contains(reply_type);
    validate_shape(object, selected)?;
    let room = object
        .get("room")
        .and_then(Value::as_str)
        .ok_or_else(|| "Iris /reply room must be a string".to_string())?;
    let thread_id = match object.get("threadId") {
        None | Some(Value::Null) => ptr::null_mut(),
        Some(Value::String(value)) => unsafe { new_string(env, value)? },
        Some(_) => return Err("Iris /reply threadId must be a string".to_string()),
    };
    let room_object = unsafe { new_string(env, room)? };
    let (enum_name, data) = if selected {
        let id = next_sequence();
        let id_text = id.to_string();
        let sentinel = format!("{}{}", policy.marker, id_text);
        let pending_reply = PendingReply {
            payload: raw,
            reply_type: reply_type.to_string(),
            room: room.to_string(),
            created: Instant::now(),
        };
        let mut values = pending()
            .lock()
            .map_err(|_| "pending reply lock failed".to_string())?;
        values.retain(|_, value| value.created.elapsed() < Duration::from_secs(120));
        values.insert(id_text, pending_reply);
        drop(values);
        ("TEXT", unsafe { make_json_primitive(env, &sentinel)? })
    } else {
        let enum_name = match reply_type {
            "text" => "TEXT",
            "image" => "IMAGE",
            "image_multiple" => "IMAGE_MULTIPLE",
            value => return Err(format!("unsupported Iris /reply type: {value}")),
        };
        if !object.contains_key("data") {
            return Err("Iris /reply data is required".to_string());
        }
        (enum_name, unsafe { json_object_get(env, decoded, "data")? })
    };
    let enum_value = unsafe { reply_type_value(env, enum_name)? };
    unsafe { new_reply_request(env, enum_value, room_object, data, thread_id) }
}

unsafe fn intercept_send(
    env: *mut JNIEnv,
    backup: jobject,
    args: jobjectArray,
) -> Result<jobject, String> {
    let length = unsafe { array_length(env, args)? };
    if length != 5 {
        return Err("unexpected Replier.sendMessage arguments".to_string());
    }
    let receiver = unsafe { array_element(env, args, 0)? };
    let message_object = unsafe { array_element(env, args, 3)? };
    let message = unsafe { object_text(env, message_object)? };
    let marker = &policy()?.marker;
    let Some(id) = message.strip_prefix(marker) else {
        return unsafe { invoke_backup(env, backup, receiver, args, 1) };
    };
    let request = pending()
        .lock()
        .map_err(|_| "pending reply lock failed".to_string())?
        .remove(id);
    let Some(request) = request else {
        return unsafe { invoke_backup(env, backup, receiver, args, 1) };
    };
    forward_reply(id.parse().unwrap_or_default(), &request)
        .map_err(|error| format!("Noa {} hook failed: {error}", request.reply_type))?;
    Ok(ptr::null_mut())
}

unsafe fn intercept_routing(
    env: *mut JNIEnv,
    backup: jobject,
    args: jobjectArray,
) -> Result<jobject, String> {
    if unsafe { array_length(env, args)? } != 3 {
        return Err("unexpected RoutingRoot.interceptor arguments".to_string());
    }
    let receiver = unsafe { array_element(env, args, 0)? };
    let pipeline_context = unsafe { array_element(env, args, 1)? };
    let completion = unsafe { array_element(env, args, 2)? };
    // Ktor can invoke the suspend-method hook with a temporarily unavailable
    // pipeline context. Endpoint inspection must never break Iris' own routes.
    if pipeline_context.is_null() || completion.is_null() {
        log_routing_inspection_bypass("routing context or continuation is null");
        return unsafe { invoke_backup(env, backup, receiver, args, 1) };
    }
    let path = match unsafe { endpoint_request_path(env, pipeline_context) } {
        Ok(path) => path,
        Err(error) => {
            log_routing_inspection_bypass(&error);
            return unsafe { invoke_backup(env, backup, receiver, args, 1) };
        }
    };
    if !ROUTING_INTERCEPTED.swap(true, Ordering::Relaxed) {
        log(LOG_INFO, &format!("Iris routing hook invoked for {path}"));
    }
    let prefix = &policy()?.endpoint_prefix;
    if path == *prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
    {
        let result = unsafe { dispatch_endpoint(env, pipeline_context, completion)? };
        if !ENDPOINT_DISPATCHED.swap(true, Ordering::Relaxed) {
            log(LOG_INFO, "Iris endpoint gateway dispatched a request");
        }
        return Ok(result);
    }
    unsafe { invoke_backup(env, backup, receiver, args, 1) }
}

fn log_routing_inspection_bypass(reason: &str) {
    if !ROUTING_INSPECTION_BYPASSED.swap(true, Ordering::Relaxed) {
        log(
            LOG_INFO,
            &format!("Iris routing inspection bypassed: {reason}"),
        );
    }
}

fn validate_shape(object: &serde_json::Map<String, Value>, selected: bool) -> Result<(), String> {
    for key in object.keys() {
        let allowed = matches!(key.as_str(), "type" | "room" | "data" | "threadId")
            || selected && key == "path";
        if !allowed {
            return Err(format!("unknown Iris /reply field: {key}"));
        }
    }
    if !matches!(object.get("room"), Some(Value::String(_))) {
        return Err("Iris /reply room must be a string".to_string());
    }
    Ok(())
}

fn notify_ready() -> Result<(), String> {
    let policy = policy()?;
    let request = BridgeRequest {
        event: "ready",
        token: &policy.token,
        pid: std::process::id(),
        id: None,
        payload: None,
        reply_type: None,
        room: None,
        method: None,
        uri: None,
        content_type: None,
        body: None,
    };
    bridge_transaction(policy.address, &request).map(|_| ())
}

fn forward_reply(id: u64, reply: &PendingReply) -> Result<(), String> {
    let policy = policy()?;
    let request = BridgeRequest {
        event: "reply",
        token: &policy.token,
        pid: std::process::id(),
        id: Some(id),
        payload: Some(&reply.payload),
        reply_type: Some(&reply.reply_type),
        room: Some(&reply.room),
        method: None,
        uri: None,
        content_type: None,
        body: None,
    };
    let response = bridge_transaction(policy.address, &request)?;
    if response.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Noa bridge rejected the request")
            .to_string())
    }
}

fn forward_endpoint(
    method: &str,
    uri: &str,
    content_type: &str,
    body: &str,
) -> Result<EndpointResponse, String> {
    let policy = policy()?;
    let request = BridgeRequest {
        event: "endpoint",
        token: &policy.token,
        pid: std::process::id(),
        id: Some(next_sequence()),
        payload: None,
        reply_type: None,
        room: None,
        method: Some(method),
        uri: Some(uri),
        content_type: Some(content_type),
        body: Some(body),
    };
    let response = bridge_transaction(policy.address, &request)?;
    if response.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Noa endpoint bridge rejected the request")
            .to_string());
    }
    let status = response
        .get("status")
        .and_then(Value::as_u64)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| "Noa endpoint bridge returned an invalid status".to_string())?;
    let content_type = response
        .get("contentType")
        .and_then(Value::as_str)
        .unwrap_or("application/json; charset=utf-8")
        .to_string();
    let body = response
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(EndpointResponse {
        status,
        content_type,
        body,
    })
}

fn bridge_transaction<T: Serialize>(address: SocketAddr, request: &T) -> Result<Value, String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(125)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    stream
        .set_nodelay(true)
        .map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut stream, request).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err("Noa bridge disconnected without a response".to_string());
    }
    serde_json::from_str(line.trim()).map_err(|error| error.to_string())
}

fn policy() -> Result<&'static Policy, String> {
    POLICY
        .get()
        .ok_or_else(|| "Iris policy is unavailable".to_string())
}

fn pending() -> &'static Mutex<HashMap<String, PendingReply>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_sequence() -> u64 {
    let mut value = SEQUENCE.lock().unwrap();
    *value = value.wrapping_add(1).max(1);
    *value
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

unsafe fn system_loader(env: *mut JNIEnv) -> Result<jobject, String> {
    let class = unsafe { find_class(env, "java/lang/ClassLoader")? };
    unsafe {
        call_static_object(
            env,
            class,
            "getSystemClassLoader",
            "()Ljava/lang/ClassLoader;",
            &[],
        )
    }
}

unsafe fn create_loader(env: *mut JNIEnv, parent: jobject) -> Result<jobject, String> {
    let buffer = unsafe {
        ((**env).v1_4.NewDirectByteBuffer)(
            env,
            ADAPTER_DEX.as_ptr().cast_mut().cast(),
            ADAPTER_DEX.len() as jlong,
        )
    };
    unsafe { check(env, "create Iris DEX buffer")? };
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
    unsafe { check(env, "retain Iris DEX loader")? };
    if global.is_null() {
        Err("Iris DEX loader global reference is null".to_string())
    } else {
        Ok(global)
    }
}

unsafe fn register_bridge(env: *mut JNIEnv, loader: jobject) -> Result<(), String> {
    let bridge = unsafe { load_class(env, loader, "dev.noa.iris.Bridge")? };
    let methods = [
        native_method(
            "invoke",
            "(ILjava/lang/reflect/Method;[Ljava/lang/Object;)Ljava/lang/Object;",
            bridge_invoke as *mut c_void,
        ),
        native_method(
            "endpoint",
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ldev/noa/iris/EndpointResponse;",
            bridge_endpoint as *mut c_void,
        ),
    ];
    let status = unsafe {
        ((**env).v1_4.RegisterNatives)(env, bridge, methods.as_ptr(), methods.len() as i32)
    };
    unsafe { check(env, "register Iris bridge")? };
    if status == JNI_OK {
        Ok(())
    } else {
        Err(format!("RegisterNatives failed: {status}"))
    }
}

unsafe fn dispatch_endpoint(
    env: *mut JNIEnv,
    pipeline_context: jobject,
    completion: jobject,
) -> Result<jobject, String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "runtime is unavailable".to_string())?;
    let installer = unsafe {
        load_class(
            env,
            runtime.loader as jobject,
            "dev.noa.iris.EndpointInstaller",
        )?
    };
    let handle = unsafe {
        ((**env).v1_4.GetStaticMethodID)(
            env,
            installer,
            c"handleFromPipeline".as_ptr(),
            c"(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;".as_ptr(),
        )
    };
    unsafe { check(env, "resolve EndpointInstaller.handleFromPipeline")? };
    let handle_arguments = [object_value(pipeline_context), object_value(completion)];
    let result = unsafe {
        ((**env).v1_4.CallStaticObjectMethodA)(env, installer, handle, handle_arguments.as_ptr())
    };
    unsafe { check(env, "dispatch Iris endpoint")? };
    Ok(result)
}

unsafe fn endpoint_request_path(
    env: *mut JNIEnv,
    pipeline_context: jobject,
) -> Result<String, String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "runtime is unavailable".to_string())?;
    let installer = unsafe {
        load_class(
            env,
            runtime.loader as jobject,
            "dev.noa.iris.EndpointInstaller",
        )?
    };
    let method = unsafe {
        ((**env).v1_4.GetStaticMethodID)(
            env,
            installer,
            c"requestPath".as_ptr(),
            c"(Ljava/lang/Object;)Ljava/lang/String;".as_ptr(),
        )
    };
    unsafe { check(env, "resolve EndpointInstaller.requestPath")? };
    let arguments = [object_value(pipeline_context)];
    let path = unsafe {
        ((**env).v1_4.CallStaticObjectMethodA)(env, installer, method, arguments.as_ptr())
    };
    unsafe { check(env, "read Iris request path")? };
    unsafe { java_string(env, path as jstring) }
}

unsafe fn new_endpoint_response(
    env: *mut JNIEnv,
    response: EndpointResponse,
) -> Result<jobject, String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "runtime is unavailable".to_string())?;
    let class = unsafe {
        load_class(
            env,
            runtime.loader as jobject,
            "dev.noa.iris.EndpointResponse",
        )?
    };
    let content_type = unsafe { new_string(env, &response.content_type)? };
    let body = unsafe { new_string(env, &response.body)? };
    unsafe {
        new_object(
            env,
            class,
            "(ILjava/lang/String;Ljava/lang/String;)V",
            &[
                int_value(response.status),
                object_value(content_type),
                object_value(body),
            ],
        )
    }
}

fn load_lsplant() -> Result<*mut c_void, String> {
    let name = CString::new("noa-lsplant").unwrap();
    let fd =
        unsafe { libc::syscall(libc::SYS_memfd_create, name.as_ptr(), libc::MFD_CLOEXEC) } as c_int;
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
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

unsafe fn app_class(env: *mut JNIEnv, name: &str) -> Result<jclass, String> {
    RUNTIME
        .get()
        .ok_or_else(|| "runtime is unavailable".to_string())?;
    let parent = unsafe { system_loader(env)? };
    unsafe { load_class(env, parent, name) }
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

unsafe fn find_method(
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
    let count = unsafe { array_length(env, methods)? };
    for index in 0..count {
        let method = unsafe { array_element(env, methods, index)? };
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
        if unsafe { array_length(env, parameters)? } as usize != parameter_types.len() {
            continue;
        }
        let mut matches = true;
        for (parameter_index, expected) in parameter_types.iter().enumerate() {
            let parameter = unsafe { array_element(env, parameters, parameter_index as i32)? };
            let actual = unsafe { class_name(env, parameter.cast())? };
            if actual != *expected {
                matches = false;
                break;
            }
        }
        if matches {
            unsafe { call_void(env, method, "setAccessible", "(Z)V", &[bool_value(true)])? };
            return Ok(method);
        }
    }
    Err(format!(
        "method {name}({}) was not found",
        parameter_types.join(", ")
    ))
}

unsafe fn class_name(env: *mut JNIEnv, class: jclass) -> Result<String, String> {
    let name = unsafe { call_object(env, class, "getName", "()Ljava/lang/String;", &[])? };
    unsafe { java_string(env, name.cast()) }
}

unsafe fn make_json_primitive(env: *mut JNIEnv, text: &str) -> Result<jobject, String> {
    let class = unsafe { app_class(env, "kotlinx.serialization.json.JsonElementKt")? };
    let method = unsafe { find_method(env, class, "JsonPrimitive", &["java.lang.String"])? };
    let value = unsafe { new_string(env, text)? };
    unsafe { invoke_reflect(env, method, ptr::null_mut(), &[value]) }
}

unsafe fn json_object_get(env: *mut JNIEnv, object: jobject, key: &str) -> Result<jobject, String> {
    let map = unsafe { find_class(env, "java/util/Map")? };
    let method = unsafe {
        ((**env).v1_4.GetMethodID)(
            env,
            map,
            c"get".as_ptr(),
            c"(Ljava/lang/Object;)Ljava/lang/Object;".as_ptr(),
        )
    };
    unsafe { check(env, "resolve JsonObject.get")? };
    let key = unsafe { new_string(env, key)? };
    let arguments = [object_value(key)];
    let result =
        unsafe { ((**env).v1_4.CallObjectMethodA)(env, object, method, arguments.as_ptr()) };
    unsafe { check(env, "read Iris data")? };
    if result.is_null() {
        Err("Iris /reply data is required".to_string())
    } else {
        Ok(result)
    }
}

unsafe fn reply_type_value(env: *mut JNIEnv, name: &str) -> Result<jobject, String> {
    let class = unsafe { app_class(env, "party.qwer.iris.model.ReplyType")? };
    unsafe { static_object(env, class, name, "Lparty/qwer/iris/model/ReplyType;") }
}

unsafe fn new_reply_request(
    env: *mut JNIEnv,
    reply_type: jobject,
    room: jobject,
    data: jobject,
    thread_id: jobject,
) -> Result<jobject, String> {
    let class = unsafe { app_class(env, "party.qwer.iris.model.ReplyRequest")? };
    unsafe {
        new_object(
            env,
            class,
            "(Lparty/qwer/iris/model/ReplyType;Ljava/lang/String;Lkotlinx/serialization/json/JsonElement;Ljava/lang/String;)V",
            &[
                object_value(reply_type),
                object_value(room),
                object_value(data),
                object_value(thread_id),
            ],
        )
    }
}

unsafe fn invoke_backup(
    env: *mut JNIEnv,
    backup: jobject,
    receiver: jobject,
    args: jobjectArray,
    offset: i32,
) -> Result<jobject, String> {
    let length = unsafe { array_length(env, args)? };
    let mut values = Vec::with_capacity((length - offset) as usize);
    for index in offset..length {
        values.push(unsafe { array_element(env, args, index)? });
    }
    unsafe { invoke_reflect(env, backup, receiver, &values) }
}

unsafe fn invoke_reflect(
    env: *mut JNIEnv,
    method: jobject,
    receiver: jobject,
    arguments: &[jobject],
) -> Result<jobject, String> {
    let array = unsafe { object_array(env, arguments)? };
    unsafe {
        call_object(
            env,
            method,
            "invoke",
            "(Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;",
            &[object_value(receiver), object_value(array)],
        )
    }
}

unsafe fn static_object(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    signature: &str,
) -> Result<jobject, String> {
    let name = CString::new(name).map_err(|_| "field name contains NUL".to_string())?;
    let signature =
        CString::new(signature).map_err(|_| "field signature contains NUL".to_string())?;
    let field =
        unsafe { ((**env).v1_4.GetStaticFieldID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve static field")? };
    let value = unsafe { ((**env).v1_4.GetStaticObjectField)(env, class, field) };
    unsafe { check(env, "read static field")? };
    if value.is_null() {
        Err("static field is null".to_string())
    } else {
        Ok(value)
    }
}

unsafe fn find_class(env: *mut JNIEnv, name: &str) -> Result<jclass, String> {
    let name = CString::new(name).map_err(|_| "class name contains NUL".to_string())?;
    let class = unsafe { ((**env).v1_4.FindClass)(env, name.as_ptr()) };
    unsafe { check(env, "find Java class")? };
    if class.is_null() {
        Err("Java class is null".to_string())
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
    let signature =
        CString::new(signature).map_err(|_| "constructor signature contains NUL".to_string())?;
    let constructor =
        unsafe { ((**env).v1_4.GetMethodID)(env, class, c"<init>".as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve constructor")? };
    let object = unsafe { ((**env).v1_4.NewObjectA)(env, class, constructor, arguments.as_ptr()) };
    unsafe { check(env, "construct Java object")? };
    if object.is_null() {
        Err("constructed Java object is null".to_string())
    } else {
        Ok(object)
    }
}

unsafe fn call_object(
    env: *mut JNIEnv,
    object: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<jobject, String> {
    let class = unsafe { ((**env).v1_4.GetObjectClass)(env, object) };
    unsafe { check(env, "resolve Java object class")? };
    let name = CString::new(name).map_err(|_| "method name contains NUL".to_string())?;
    let signature =
        CString::new(signature).map_err(|_| "method signature contains NUL".to_string())?;
    let method =
        unsafe { ((**env).v1_4.GetMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve Java method")? };
    let result =
        unsafe { ((**env).v1_4.CallObjectMethodA)(env, object, method, arguments.as_ptr()) };
    unsafe { check(env, "call Java method")? };
    Ok(result)
}

unsafe fn call_static_object(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<jobject, String> {
    let name = CString::new(name).map_err(|_| "method name contains NUL".to_string())?;
    let signature =
        CString::new(signature).map_err(|_| "method signature contains NUL".to_string())?;
    let method =
        unsafe { ((**env).v1_4.GetStaticMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve static Java method")? };
    let result =
        unsafe { ((**env).v1_4.CallStaticObjectMethodA)(env, class, method, arguments.as_ptr()) };
    unsafe { check(env, "call static Java method")? };
    Ok(result)
}

unsafe fn call_void(
    env: *mut JNIEnv,
    object: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<(), String> {
    let class = unsafe { ((**env).v1_4.GetObjectClass)(env, object) };
    unsafe { check(env, "resolve Java object class")? };
    let name = CString::new(name).map_err(|_| "method name contains NUL".to_string())?;
    let signature =
        CString::new(signature).map_err(|_| "method signature contains NUL".to_string())?;
    let method =
        unsafe { ((**env).v1_4.GetMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve Java void method")? };
    unsafe { ((**env).v1_4.CallVoidMethodA)(env, object, method, arguments.as_ptr()) };
    unsafe { check(env, "call Java void method") }
}

unsafe fn object_array(env: *mut JNIEnv, values: &[jobject]) -> Result<jobjectArray, String> {
    let class = unsafe { find_class(env, "java/lang/Object")? };
    let array =
        unsafe { ((**env).v1_4.NewObjectArray)(env, values.len() as i32, class, ptr::null_mut()) };
    unsafe { check(env, "create object array")? };
    for (index, value) in values.iter().enumerate() {
        unsafe { ((**env).v1_4.SetObjectArrayElement)(env, array, index as i32, *value) };
        unsafe { check(env, "write object array")? };
    }
    Ok(array)
}

unsafe fn array_length(env: *mut JNIEnv, array: jobjectArray) -> Result<i32, String> {
    let length = unsafe { ((**env).v1_4.GetArrayLength)(env, array) };
    unsafe { check(env, "read array length")? };
    Ok(length)
}

unsafe fn array_element(
    env: *mut JNIEnv,
    array: jobjectArray,
    index: i32,
) -> Result<jobject, String> {
    let value = unsafe { ((**env).v1_4.GetObjectArrayElement)(env, array, index) };
    unsafe { check(env, "read array element")? };
    Ok(value)
}

unsafe fn object_text(env: *mut JNIEnv, object: jobject) -> Result<String, String> {
    if object.is_null() {
        return Err("Java object is null".to_string());
    }
    let text = unsafe { call_object(env, object, "toString", "()Ljava/lang/String;", &[])? };
    unsafe { java_string(env, text.cast()) }
}

unsafe fn new_string(env: *mut JNIEnv, value: &str) -> Result<jobject, String> {
    let utf16 = value.encode_utf16().collect::<Vec<_>>();
    let string = unsafe { ((**env).v1_4.NewString)(env, utf16.as_ptr(), utf16.len() as i32) };
    unsafe { check(env, "create Java string")? };
    if string.is_null() {
        Err("Java string is null".to_string())
    } else {
        Ok(string.cast())
    }
}

unsafe fn java_string(env: *mut JNIEnv, value: jstring) -> Result<String, String> {
    if value.is_null() {
        return Err("Java string is null".to_string());
    }
    let length = unsafe { ((**env).v1_4.GetStringLength)(env, value) };
    let chars = unsafe { ((**env).v1_4.GetStringChars)(env, value, ptr::null_mut()) };
    unsafe { check(env, "read Java string")? };
    if chars.is_null() {
        return Err("Java string characters are null".to_string());
    }
    let text =
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(chars, length as usize) });
    unsafe { ((**env).v1_4.ReleaseStringChars)(env, value, chars) };
    Ok(text)
}

unsafe fn check(env: *mut JNIEnv, context: &str) -> Result<(), String> {
    if !unsafe { ((**env).v1_4.ExceptionCheck)(env) } {
        return Ok(());
    }
    let exception = unsafe { ((**env).v1_4.ExceptionOccurred)(env) };
    unsafe { ((**env).v1_4.ExceptionClear)(env) };
    let detail = if exception.is_null() {
        "unknown Java exception".to_string()
    } else {
        unsafe { exception_text(env, exception) }.unwrap_or_else(|_| "Java exception".to_string())
    };
    Err(format!("{context}: {detail}"))
}

unsafe fn exception_text(env: *mut JNIEnv, exception: jobject) -> Result<String, String> {
    let mut current = exception;
    let mut details = Vec::new();
    for _ in 0..6 {
        if current.is_null() {
            break;
        }
        details.push(unsafe { object_text(env, current)? });
        current = unsafe { call_object(env, current, "getCause", "()Ljava/lang/Throwable;", &[])? };
    }
    Ok(details.join(": caused by "))
}

unsafe fn throw_runtime(env: *mut JNIEnv, message: &str) {
    let Ok(class) = (unsafe { find_class(env, "java/lang/RuntimeException") }) else {
        return;
    };
    let message = CString::new(message.replace('\0', " ")).unwrap();
    unsafe { ((**env).v1_4.ThrowNew)(env, class, message.as_ptr()) };
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

fn int_value(value: i32) -> jvalue {
    jvalue { i: value }
}

fn bool_value(value: bool) -> jvalue {
    jvalue { z: value }
}

fn log(priority: c_int, message: &str) {
    let tag = c"NoaIrisAgent";
    let message = CString::new(message.replace('\0', " ")).unwrap();
    unsafe { __android_log_write(priority, tag.as_ptr(), message.as_ptr()) };
}
