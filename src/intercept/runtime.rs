use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_int, c_uint, c_void},
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;
use tracing::{error, info, warn};

use super::{record_loco_packet, set_active, set_kakao_active};
use crate::{failure::NoaError, model::LocoPacket, settings::Settings};

#[repr(C)]
struct GError {
    domain: c_uint,
    code: c_int,
    message: *mut c_char,
}

enum FridaDeviceManager {}
enum FridaDevice {}
enum GMainContext {}
enum GCancellable {}
enum GBytes {}

unsafe extern "C" {
    fn frida_init();
    fn frida_deinit();
    fn frida_version_string() -> *const c_char;
    fn frida_unref(value: *mut c_void);
    fn frida_get_main_context() -> *mut GMainContext;
    fn frida_device_manager_new() -> *mut FridaDeviceManager;
    fn frida_device_manager_close_sync(
        manager: *mut FridaDeviceManager,
        cancellable: *mut GCancellable,
        error: *mut *mut GError,
    );
    fn frida_device_manager_get_device_by_type_sync(
        manager: *mut FridaDeviceManager,
        device_type: c_int,
        timeout: c_int,
        cancellable: *mut GCancellable,
        error: *mut *mut GError,
    ) -> *mut FridaDevice;
    fn frida_device_inject_library_blob_sync(
        device: *mut FridaDevice,
        pid: c_uint,
        blob: *mut GBytes,
        entrypoint: *const c_char,
        data: *const c_char,
        cancellable: *mut GCancellable,
        error: *mut *mut GError,
    ) -> c_uint;
    #[link_name = "_frida_g_main_context_iteration"]
    fn g_main_context_iteration(context: *mut GMainContext, may_block: c_int) -> c_int;
    #[link_name = "_frida_g_error_free"]
    fn g_error_free(error: *mut GError);
    #[link_name = "_frida_g_bytes_new_static"]
    fn g_bytes_new_static(data: *const c_void, size: usize) -> *mut GBytes;
    #[link_name = "_frida_g_bytes_unref"]
    fn g_bytes_unref(bytes: *mut GBytes);
}

const KAKAO_AGENT: &[u8] = include_bytes!(env!("NOA_KAKAO_AGENT_BLOB"));
const IRIS_AGENT: &[u8] = include_bytes!(env!("NOA_IRIS_AGENT_BLOB"));

struct KakaoCommand {
    id: u64,
    room_id: i64,
    action: KakaoAction,
}

enum KakaoAction {
    SendCustom { row_id: i64 },
    KickMember { user_id: i64 },
    ChatOnRoom,
}

#[derive(Clone, Copy)]
enum NativeKind {
    Iris,
    Kakao,
}

impl NativeKind {
    fn label(self) -> &'static str {
        match self {
            Self::Iris => "Iris",
            Self::Kakao => "KakaoTalk",
        }
    }

    fn active(self) -> bool {
        match self {
            Self::Iris => super::active(),
            Self::Kakao => super::kakao_active(),
        }
    }

    fn set_active(self, value: bool) {
        match self {
            Self::Iris => set_active(value),
            Self::Kakao => set_kakao_active(value),
        }
    }
}

struct NativeInjection {
    pid: u32,
    injected_at: Instant,
}

struct NativeConnection {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    pid: u32,
}

static COMMAND_SENDER: OnceLock<mpsc::Sender<KakaoCommand>> = OnceLock::new();
type PendingResponse = mpsc::SyncSender<Result<(), String>>;
static PENDING: OnceLock<Mutex<HashMap<u64, PendingResponse>>> = OnceLock::new();
static NEXT_COMMAND_ID: AtomicU64 = AtomicU64::new(1);

pub fn launch(config: Arc<Settings>) {
    if let Err(error) = thread::Builder::new()
        .name("noa-intercept".to_string())
        .spawn(move || run(config))
    {
        error!(%error, "네이티브 후킹 스레드를 시작하지 못했습니다");
    }
}

pub async fn send_custom(room_id: i64, row_id: i64) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(room_id, KakaoAction::SendCustom { row_id })
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))?
}

pub async fn kick_member(room_id: i64, user_id: i64) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(room_id, KakaoAction::KickMember { user_id })
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))?
}

pub async fn chat_on_room(room_id: i64) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || send_command_blocking(room_id, KakaoAction::ChatOnRoom))
        .await
        .map_err(|error| NoaError::Internal(error.to_string()))?
}

fn send_command_blocking(room_id: i64, action: KakaoAction) -> Result<(), NoaError> {
    if !super::kakao_active() {
        return Err(NoaError::AndroidUnavailable(
            "KakaoTalk 후킹 에이전트가 준비되지 않았습니다".to_string(),
        ));
    }
    let sender = COMMAND_SENDER.get().ok_or_else(|| {
        NoaError::AndroidUnavailable("네이티브 명령 채널이 준비되지 않았습니다".to_string())
    })?;
    let id = NEXT_COMMAND_ID.fetch_add(1, Ordering::Relaxed);
    let (response_sender, response_receiver) = mpsc::sync_channel(1);
    pending()
        .lock()
        .map_err(|_| NoaError::Internal("네이티브 응답 잠금이 손상되었습니다".to_string()))?
        .insert(id, response_sender);
    if sender
        .send(KakaoCommand {
            id,
            room_id,
            action,
        })
        .is_err()
    {
        remove_pending(id);
        return Err(NoaError::AndroidUnavailable(
            "네이티브 명령 채널이 종료되었습니다".to_string(),
        ));
    }
    match response_receiver.recv_timeout(Duration::from_secs(12)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => Err(NoaError::AndroidUnavailable(message)),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            remove_pending(id);
            Err(NoaError::AndroidUnavailable(
                "KakaoTalk 후킹 발신 호출 시간이 초과되었습니다".to_string(),
            ))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(NoaError::AndroidUnavailable(
            "KakaoTalk 후킹 발신 응답 채널이 종료되었습니다".to_string(),
        )),
    }
}

fn run(config: Arc<Settings>) {
    unsafe {
        let kakao_listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) => {
                error!(%error, "KakaoTalk 네이티브 명령 채널을 열지 못했습니다");
                return;
            }
        };
        let iris_listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) => {
                error!(%error, "Iris 네이티브 브리지 채널을 열지 못했습니다");
                return;
            }
        };
        let kakao_event_listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) => {
                error!(%error, "KakaoTalk LOCO 이벤트 채널을 열지 못했습니다");
                return;
            }
        };
        let kakao_port = match kakao_listener.local_addr() {
            Ok(address) => address.port(),
            Err(error) => {
                error!(%error, "KakaoTalk 네이티브 명령 포트를 확인하지 못했습니다");
                return;
            }
        };
        let iris_port = match iris_listener.local_addr() {
            Ok(address) => address.port(),
            Err(error) => {
                error!(%error, "Iris 네이티브 브리지 포트를 확인하지 못했습니다");
                return;
            }
        };
        let kakao_event_port = match kakao_event_listener.local_addr() {
            Ok(address) => address.port(),
            Err(error) => {
                error!(%error, "KakaoTalk LOCO 이벤트 포트를 확인하지 못했습니다");
                return;
            }
        };
        let (command_sender, command_receiver) = mpsc::channel();
        if COMMAND_SENDER.set(command_sender).is_err() {
            error!("KakaoTalk 네이티브 명령 채널을 초기화하지 못했습니다");
            return;
        }
        launch_kakao_bridge(
            kakao_listener,
            command_receiver,
            config.iris_hook.token.clone(),
        );
        launch_kakao_event_bridge(kakao_event_listener, config.iris_hook.token.clone());
        launch_iris_bridge(
            iris_listener,
            config.iris_hook.token.clone(),
            config.iris_hook.bridge_url.clone(),
        );

        frida_init();
        let version = string_from_pointer(frida_version_string());
        info!(%version, "내장 Frida Core 초기화 완료");
        let manager = frida_device_manager_new();
        if manager.is_null() {
            error!("Frida DeviceManager를 만들지 못했습니다");
            frida_deinit();
            return;
        }
        let mut failure = ptr::null_mut();
        let device = frida_device_manager_get_device_by_type_sync(
            manager,
            0,
            5_000,
            ptr::null_mut(),
            &mut failure,
        );
        if device.is_null() {
            error!(error = %take_error(failure), "Frida 로컬 장치를 열지 못했습니다");
            close_manager(manager);
            frida_deinit();
            return;
        }

        let mut iris_injection = None;
        let mut kakao_injection = None;
        let mut iris_target = None;
        let mut kakao_target = None;
        let mut next_scan = Instant::now();
        let mut iris_retry = Instant::now();
        let mut kakao_retry = Instant::now();
        loop {
            pump();
            let now = Instant::now();
            if now >= next_scan {
                iris_target = config.iris_hook.enabled.then(iris_process_pid).flatten();
                kakao_target = config.kakao_hook_enabled.then(kakao_process_pid).flatten();
                next_scan = now + Duration::from_millis(500);
            }
            refresh_native_agent(
                device,
                &mut iris_injection,
                iris_target,
                NativeKind::Iris,
                iris_port,
                kakao_event_port,
                &config,
                &mut iris_retry,
            );
            refresh_native_agent(
                device,
                &mut kakao_injection,
                kakao_target,
                NativeKind::Kakao,
                kakao_port,
                kakao_event_port,
                &config,
                &mut kakao_retry,
            );
            pump();
            thread::sleep(Duration::from_millis(25));
        }
    }
}

unsafe fn refresh_native_agent(
    device: *mut FridaDevice,
    slot: &mut Option<NativeInjection>,
    target: Option<u32>,
    kind: NativeKind,
    port: u16,
    event_port: u16,
    config: &Settings,
    retry_at: &mut Instant,
) {
    let changed = slot
        .as_ref()
        .is_some_and(|injection| target != Some(injection.pid));
    if (changed || target.is_none()) && slot.take().is_some() {
        kind.set_active(false);
        if matches!(kind, NativeKind::Kakao) {
            fail_pending("KakaoTalk 네이티브 후킹 연결이 종료되었습니다");
        }
    }
    let stalled = slot.as_ref().is_some_and(|injection| {
        !kind.active() && injection.injected_at.elapsed() >= Duration::from_secs(15)
    });
    if stalled {
        *slot = None;
    }
    if slot.is_some() || target.is_none() || Instant::now() < *retry_at {
        return;
    }
    let pid = target.unwrap();
    kind.set_active(false);
    match unsafe { inject_native_agent(device, pid, kind, port, event_port, config) } {
        Ok(injection_id) => {
            info!(
                pid,
                injection_id,
                process = kind.label(),
                "Rust 에이전트 주입 완료"
            );
            *slot = Some(NativeInjection {
                pid,
                injected_at: Instant::now(),
            });
        }
        Err(message) => {
            warn!(pid, process = kind.label(), error = %message, "Rust 에이전트 주입 실패");
            *retry_at = Instant::now() + Duration::from_secs(2);
        }
    }
}

unsafe fn inject_native_agent(
    device: *mut FridaDevice,
    pid: u32,
    kind: NativeKind,
    port: u16,
    event_port: u16,
    config: &Settings,
) -> Result<u32, String> {
    let (agent, entrypoint, data) = match kind {
        NativeKind::Iris => (
            IRIS_AGENT,
            c"noa_iris_agent_main",
            serde_json::json!({
                "port": port,
                "token": config.iris_hook.token,
                "types": config.iris_hook.types,
            }),
        ),
        NativeKind::Kakao => (
            KAKAO_AGENT,
            c"noa_agent_main",
            serde_json::json!({
                "port": port,
                "event_port": event_port,
                "token": config.iris_hook.token,
            }),
        ),
    };
    let blob = unsafe { g_bytes_new_static(agent.as_ptr().cast(), agent.len()) };
    if blob.is_null() {
        return Err("Frida GBytes 생성 실패".to_string());
    }
    let data = CString::new(data.to_string()).map_err(|_| {
        format!(
            "{} 에이전트 초기화 데이터가 올바르지 않습니다",
            kind.label()
        )
    })?;
    let mut failure = ptr::null_mut();
    let injection_id = unsafe {
        frida_device_inject_library_blob_sync(
            device,
            pid,
            blob,
            entrypoint.as_ptr(),
            data.as_ptr(),
            ptr::null_mut(),
            &mut failure,
        )
    };
    unsafe { g_bytes_unref(blob) };
    if injection_id == 0 || !failure.is_null() {
        Err(unsafe { take_error(failure) })
    } else {
        Ok(injection_id)
    }
}

fn launch_kakao_bridge(
    listener: TcpListener,
    commands: mpsc::Receiver<KakaoCommand>,
    token: String,
) {
    if let Err(error) = listener.set_nonblocking(true) {
        error!(%error, "KakaoTalk 네이티브 명령 수신기를 설정하지 못했습니다");
        return;
    }
    if let Err(error) = thread::Builder::new()
        .name("noa-kakao-bridge".to_string())
        .spawn(move || kakao_bridge_loop(listener, commands, token))
    {
        error!(%error, "KakaoTalk 네이티브 명령 스레드를 시작하지 못했습니다");
    }
}

fn kakao_bridge_loop(listener: TcpListener, commands: mpsc::Receiver<KakaoCommand>, token: String) {
    let mut connection = None;
    loop {
        if connection.is_some() && !super::kakao_active() {
            connection = None;
        }
        if connection.is_none() {
            match listener.accept() {
                Ok((stream, _)) => match accept_kakao_connection(stream, &token) {
                    Ok(accepted) => {
                        let pid = accepted.pid;
                        connection = Some(accepted);
                        set_kakao_active(true);
                        info!(pid, "KakaoTalk Rust 에이전트 준비 완료");
                    }
                    Err(message) => warn!(error = %message, "KakaoTalk Rust 에이전트 연결 거부"),
                },
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => warn!(%error, "KakaoTalk 네이티브 연결 수락 실패"),
            }
        }
        match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(command) => {
                let Some(active) = connection.as_mut() else {
                    complete_pending(
                        command.id,
                        Err("KakaoTalk Rust 에이전트가 연결되지 않았습니다".to_string()),
                    );
                    continue;
                };
                if let Err(message) = transact_kakao(active, &token, &command) {
                    complete_pending(command.id, Err(message.clone()));
                    warn!(pid = active.pid, error = %message, "KakaoTalk Rust 에이전트 연결 종료");
                    connection = None;
                    set_kakao_active(false);
                    fail_pending("KakaoTalk Rust 에이전트 연결이 종료되었습니다");
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn accept_kakao_connection(stream: TcpStream, token: &str) -> Result<NativeConnection, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    stream
        .set_nodelay(true)
        .map_err(|error| error.to_string())?;
    let reader_stream = stream.try_clone().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(reader_stream);
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err("준비 응답 없이 연결이 종료되었습니다".to_string());
    }
    let hello: Value = serde_json::from_str(line.trim()).map_err(|error| error.to_string())?;
    if hello.get("event").and_then(Value::as_str) != Some("ready")
        || hello.get("token").and_then(Value::as_str) != Some(token)
        || hello.get("protocol").and_then(Value::as_u64) != Some(1)
    {
        return Err("네이티브 에이전트 인증 또는 프로토콜이 올바르지 않습니다".to_string());
    }
    let pid = hello
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| "네이티브 에이전트 PID가 없습니다".to_string())?;
    Ok(NativeConnection {
        stream,
        reader,
        pid,
    })
}

fn transact_kakao(
    connection: &mut NativeConnection,
    token: &str,
    command: &KakaoCommand,
) -> Result<(), String> {
    let request = match &command.action {
        KakaoAction::SendCustom { row_id } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "send-custom",
            "room": command.room_id,
            "row": row_id,
        }),
        KakaoAction::KickMember { user_id } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "kick-member",
            "room": command.room_id,
            "user": user_id,
        }),
        KakaoAction::ChatOnRoom => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "chat-on-room",
            "room": command.room_id,
        }),
    };
    connection
        .stream
        .set_read_timeout(Some(Duration::from_secs(11)))
        .map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut connection.stream, &request).map_err(|error| error.to_string())?;
    connection
        .stream
        .write_all(b"\n")
        .map_err(|error| error.to_string())?;
    connection
        .stream
        .flush()
        .map_err(|error| error.to_string())?;
    let mut line = String::new();
    if connection
        .reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err("네이티브 에이전트 응답 없이 연결이 종료되었습니다".to_string());
    }
    let response: Value = serde_json::from_str(line.trim()).map_err(|error| error.to_string())?;
    if response.get("id").and_then(Value::as_u64) != Some(command.id) {
        return Err("네이티브 에이전트 응답 ID가 일치하지 않습니다".to_string());
    }
    let result = if response.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("KakaoTalk Rust 에이전트 호출 실패")
            .to_string())
    };
    complete_pending(command.id, result);
    Ok(())
}

fn launch_kakao_event_bridge(listener: TcpListener, token: String) {
    if let Err(error) = thread::Builder::new()
        .name("noa-kakao-loco".to_string())
        .spawn(move || {
            for incoming in listener.incoming() {
                match incoming {
                    Ok(stream) => {
                        let token = token.clone();
                        let _ = thread::Builder::new()
                            .name("noa-kakao-loco-stream".to_string())
                            .spawn(move || read_kakao_events(stream, &token));
                    }
                    Err(error) => warn!(%error, "KakaoTalk LOCO 이벤트 연결 수락 실패"),
                }
            }
        })
    {
        error!(%error, "KakaoTalk LOCO 이벤트 스레드를 시작하지 못했습니다");
    }
}

fn read_kakao_events(stream: TcpStream, token: &str) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) => {
                warn!(%error, "KakaoTalk LOCO 이벤트 스트림 종료");
                return;
            }
        }
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if value.get("token").and_then(Value::as_str) != Some(token)
            || value.get("event").and_then(Value::as_str) != Some("loco")
        {
            continue;
        }
        record_loco_packet(LocoPacket {
            id: NEXT_COMMAND_ID.fetch_add(1, Ordering::Relaxed),
            direction: value
                .get("direction")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            method: value
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("NONE")
                .to_string(),
            packet_id: value
                .get("packetId")
                .and_then(Value::as_i64)
                .unwrap_or_default() as i32,
            status: value
                .get("status")
                .and_then(Value::as_i64)
                .unwrap_or_default() as i16,
            body_length: value
                .get("bodyLength")
                .and_then(Value::as_i64)
                .unwrap_or_default() as i32,
            body: value
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            captured_at: value
                .get("capturedAt")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
        });
    }
}

fn launch_iris_bridge(listener: TcpListener, token: String, bridge_url: String) {
    if let Err(error) = thread::Builder::new()
        .name("noa-iris-bridge".to_string())
        .spawn(move || iris_bridge_loop(listener, token, bridge_url))
    {
        error!(%error, "Iris 네이티브 브리지 스레드를 시작하지 못했습니다");
    }
}

fn iris_bridge_loop(listener: TcpListener, token: String, bridge_url: String) {
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let token = token.clone();
                let bridge_url = bridge_url.clone();
                let _ = thread::Builder::new()
                    .name("noa-iris-request".to_string())
                    .spawn(move || {
                        if let Err(message) = handle_iris_connection(stream, &token, &bridge_url) {
                            warn!(error = %message, "Iris Rust 에이전트 요청 실패");
                        }
                    });
            }
            Err(error) => warn!(%error, "Iris 네이티브 연결 수락 실패"),
        }
    }
}

fn handle_iris_connection(
    mut stream: TcpStream,
    token: &str,
    bridge_url: &str,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(125)))
        .map_err(|error| error.to_string())?;
    stream
        .set_nodelay(true)
        .map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err("Iris 에이전트 요청이 비어 있습니다".to_string());
    }
    let request: Value = serde_json::from_str(line.trim()).map_err(|error| error.to_string())?;
    if request.get("token").and_then(Value::as_str) != Some(token) {
        write_iris_response(&mut stream, None, Err("authentication failed".to_string()))?;
        return Ok(());
    }
    let id = request.get("id").and_then(Value::as_u64);
    match request.get("event").and_then(Value::as_str) {
        Some("ready") => {
            let pid = request
                .get("pid")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            set_active(true);
            info!(pid, "Iris Rust 에이전트 준비 완료");
            write_iris_response(&mut stream, None, Ok(()))?;
        }
        Some("reply") => {
            let payload = request
                .get("payload")
                .and_then(Value::as_str)
                .ok_or_else(|| "Iris reply payload가 없습니다".to_string())?;
            let reply_type = request
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let room = request
                .get("room")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let result = forward_iris_http(bridge_url, token, payload);
            if result.is_ok() {
                info!(reply_type, room, "Iris /reply 선택 처리");
            }
            write_iris_response(&mut stream, id, result)?;
        }
        _ => write_iris_response(&mut stream, id, Err("unknown Iris event".to_string()))?,
    }
    Ok(())
}

fn write_iris_response(
    stream: &mut TcpStream,
    id: Option<u64>,
    result: Result<(), String>,
) -> Result<(), String> {
    let response = match result {
        Ok(()) => serde_json::json!({"id": id, "ok": true}),
        Err(message) => serde_json::json!({"id": id, "ok": false, "error": message}),
    };
    serde_json::to_writer(&mut *stream, &response).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    Ok(())
}

fn forward_iris_http(url: &str, token: &str, payload: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|error| error.to_string())?;
    if parsed.scheme() != "http" {
        return Err("Iris 내부 브리지는 http URL만 지원합니다".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "Iris 내부 브리지 호스트가 없습니다".to_string())?;
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "Iris 내부 브리지 포트가 없습니다".to_string())?;
    let address = (host, port)
        .to_socket_addrs()
        .map_err(|error| error.to_string())?
        .next()
        .ok_or_else(|| "Iris 내부 브리지 주소를 찾지 못했습니다".to_string())?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    let mut path = parsed.path().to_string();
    if path.is_empty() {
        path.push('/');
    }
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json; charset=utf-8\r\nX-Noa-Hook-Token: {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .map_err(|error| error.to_string())?;
    stream
        .write_all(payload.as_bytes())
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| error.to_string())?;
    let response = String::from_utf8_lossy(&response);
    let mut lines = response.lines();
    let status_line = lines.next().ok_or_else(|| "빈 HTTP 응답".to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| format!("잘못된 HTTP 상태행: {status_line}"))?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        let body = response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.trim())
            .unwrap_or_default();
        Err(format!(
            "Noa bridge returned HTTP {status}{}",
            if body.is_empty() {
                String::new()
            } else {
                format!(": {body}")
            }
        ))
    }
}

fn iris_process_pid() -> Option<u32> {
    let entries = fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(command) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let mut arguments = command
            .split(|byte| *byte == 0)
            .filter(|value| !value.is_empty());
        let Some(executable) = arguments.next() else {
            continue;
        };
        if executable.ends_with(b"app_process")
            && arguments.any(|value| value == b"party.qwer.iris.Main")
        {
            return Some(pid);
        }
    }
    None
}

fn kakao_process_pid() -> Option<u32> {
    let entries = fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(command) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if command.split(|byte| *byte == 0).next() == Some(b"com.kakao.talk") {
            return Some(pid);
        }
    }
    None
}

fn pending() -> &'static Mutex<HashMap<u64, PendingResponse>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn complete_pending(id: u64, result: Result<(), String>) {
    let sender = pending()
        .lock()
        .ok()
        .and_then(|mut values| values.remove(&id));
    if let Some(sender) = sender {
        let _ = sender.send(result);
    }
}

fn remove_pending(id: u64) {
    if let Ok(mut values) = pending().lock() {
        values.remove(&id);
    }
}

fn fail_pending(message: &str) {
    let senders = pending()
        .lock()
        .map(|mut values| values.drain().map(|(_, sender)| sender).collect::<Vec<_>>())
        .unwrap_or_default();
    for sender in senders {
        let _ = sender.send(Err(message.to_string()));
    }
}

unsafe fn close_manager(manager: *mut FridaDeviceManager) {
    unsafe { frida_device_manager_close_sync(manager, ptr::null_mut(), ptr::null_mut()) };
    unsafe { frida_unref(manager.cast()) };
}

unsafe fn pump() {
    let context = unsafe { frida_get_main_context() };
    for _ in 0..64 {
        if unsafe { g_main_context_iteration(context, 0) } == 0 {
            break;
        }
    }
}

unsafe fn take_error(error: *mut GError) -> String {
    if error.is_null() {
        return "알 수 없는 Frida Core 오류".to_string();
    }
    let message = unsafe { string_from_pointer((*error).message) };
    unsafe { g_error_free(error) };
    message
}

unsafe fn string_from_pointer(value: *const c_char) -> String {
    if value.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(value) }
        .to_string_lossy()
        .into_owned()
}
