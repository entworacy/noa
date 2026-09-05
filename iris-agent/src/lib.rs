mod java;
pub(crate) use java::{
    array_element, array_length, call_static_object, check, find_class, find_method, int_value,
    invoke_reflect, java_byte_array, java_string, load_class, native_method, new_object,
    new_string, object_text, object_value, static_object, throw_runtime,
};
use noa_agent_runtime::{
    jvm::{locate_vm, with_attached},
    lsplant::{
        initialization_error as lsplant_initialization_error, noa_lsplant_hook, noa_lsplant_init,
        noa_lsplant_uses_shorty_fallback,
    },
};
use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString, c_char, c_int, c_void},
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    ptr,
    sync::{
        Mutex, Once, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use jni::sys::{JNI_OK, JNIEnv, jbyteArray, jclass, jint, jlong, jobject, jobjectArray, jstring};
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
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

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
    #[serde(rename = "bodyEncoding", skip_serializing_if = "Option::is_none")]
    body_encoding: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<&'a str>,
}

#[derive(Serialize)]
struct FailureRequest<'a> {
    event: &'static str,
    token: &'a str,
    pid: u32,
    error: &'a str,
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
        Err(error) => {
            log(
                LOG_ERROR,
                &format!("Iris native hook initialization failed: {error}"),
            );
            if let Err(report_error) = notify_failure(&error) {
                log(
                    LOG_ERROR,
                    &format!("Iris failure notification failed: {report_error}"),
                );
            }
        }
    }
}

fn initialize_runtime() -> Result<(), String> {
    let vm = unsafe { locate_vm() }?;
    unsafe {
        with_attached(vm, |env| {
            let parent = system_loader(env)?;
            let loader = create_loader(env, parent)?;
            register_bridge(env, loader)?;
            let lsplant = load_lsplant()?;
            if !noa_lsplant_init(env, lsplant) {
                return Err(lsplant_initialization_error());
            }
            if noa_lsplant_uses_shorty_fallback() {
                log(
                    LOG_INFO,
                    "LSPlant ART GetMethodShorty compatibility fallback active",
                );
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
    body: jbyteArray,
) -> jobject {
    let result = (|| {
        let method = unsafe { java_string(env, method)? };
        let uri = unsafe { java_string(env, uri)? };
        let content_type = unsafe { java_string(env, content_type)? };
        let body = unsafe { java_byte_array(env, body)? };
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
        body_encoding: None,
        body: None,
    };
    bridge_transaction(policy.address, &request).map(|_| ())
}

fn notify_failure(error: &str) -> Result<(), String> {
    let policy = policy()?;
    let request = FailureRequest {
        event: "error",
        token: &policy.token,
        pid: std::process::id(),
        error,
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
        body_encoding: None,
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
    body: &[u8],
) -> Result<EndpointResponse, String> {
    let policy = policy()?;
    let encoded = STANDARD.encode(body);
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
        body_encoding: Some("base64"),
        body: Some(&encoded),
    };
    let read_timeout = if uri
        .split('?')
        .next()
        .is_some_and(|path| path.ends_with("/vox/audio/stream"))
    {
        Duration::from_secs(6 * 60 * 60)
    } else {
        Duration::from_secs(125)
    };
    let response = bridge_transaction_with_timeout(policy.address, &request, read_timeout)?;
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
    bridge_transaction_with_timeout(address, request, Duration::from_secs(125))
}

fn bridge_transaction_with_timeout<T: Serialize>(
    address: SocketAddr,
    request: &T,
    read_timeout: Duration,
) -> Result<Value, String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(3))
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(read_timeout))
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
            "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;[B)Ldev/noa/iris/EndpointResponse;",
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
    noa_agent_runtime::lsplant::load(LSPLANT, c"noa-lsplant")
}

unsafe fn app_class(env: *mut JNIEnv, name: &str) -> Result<jclass, String> {
    RUNTIME
        .get()
        .ok_or_else(|| "runtime is unavailable".to_string())?;
    let parent = unsafe { system_loader(env)? };
    unsafe { load_class(env, parent, name) }
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

fn log(priority: c_int, message: &str) {
    let tag = c"NoaIrisAgent";
    let message = CString::new(message.replace('\0', " ")).unwrap();
    unsafe { __android_log_write(priority, tag.as_ptr(), message.as_ptr()) };
}
