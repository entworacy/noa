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

use serde::Deserialize;
use serde_json::Value;
use tracing::{error, info, warn};

use super::{
    DatabaseInvalidation, NativeInjectionRetry, record_database_invalidation, record_loco_packet,
    set_active, set_kakao_active,
};
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
    fn frida_selinux_patch_policy();
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
const MAX_IRIS_HTTP_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

struct KakaoCommand {
    id: u64,
    room_id: i64,
    action: KakaoAction,
}

enum KakaoAction {
    SendCustom {
        row_id: i64,
    },
    KickMember {
        user_id: i64,
    },
    ChatOnRoom,
    LoadOpenChatMember {
        user_id: i64,
    },
    ShareOpenProfile,
    JoinOpenChat {
        url: String,
        profile_id: String,
        profile_kind: String,
        nickname: String,
        profile_image_url: Option<String>,
    },
}

impl KakaoAction {
    fn label(&self) -> &'static str {
        match self {
            Self::SendCustom { .. } => "custom 발신",
            Self::KickMember { .. } => "참여자 강퇴",
            Self::ChatOnRoom => "CHATONROOM",
            Self::LoadOpenChatMember { .. } => "오픈채팅 멤버 프로필 조회",
            Self::ShareOpenProfile => "오픈프로필 공유 링크 조회",
            Self::JoinOpenChat { .. } => "오픈채팅 입장",
        }
    }

    fn timeout(&self) -> Duration {
        match self {
            Self::JoinOpenChat { .. } => Duration::from_secs(35),
            _ => Duration::from_secs(12),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenChatJoinResult {
    pub room_name: String,
    pub profile_applied: bool,
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
type PendingResponse = mpsc::SyncSender<Result<Option<String>, String>>;
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
    .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

pub async fn kick_member(room_id: i64, user_id: i64) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(room_id, KakaoAction::KickMember { user_id })
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

pub async fn chat_on_room(room_id: i64) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || send_command_blocking(room_id, KakaoAction::ChatOnRoom))
        .await
        .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

pub async fn load_open_chat_member(room_id: i64, user_id: i64) -> Result<(), NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(room_id, KakaoAction::LoadOpenChatMember { user_id })
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??;
    Ok(())
}

pub async fn share_open_profile(link_id: i64) -> Result<String, NoaError> {
    tokio::task::spawn_blocking(move || {
        send_command_blocking(link_id, KakaoAction::ShareOpenProfile)
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??
    .ok_or_else(|| {
        NoaError::AndroidUnavailable(
            "KakaoTalk 후킹 에이전트가 공유 링크를 반환하지 않았습니다".to_string(),
        )
    })
}

pub async fn join_open_chat(
    url: String,
    profile_id: String,
    profile_kind: String,
    nickname: String,
    profile_image_url: Option<String>,
) -> Result<OpenChatJoinResult, NoaError> {
    let value = tokio::task::spawn_blocking(move || {
        send_command_blocking(
            0,
            KakaoAction::JoinOpenChat {
                url,
                profile_id,
                profile_kind,
                nickname,
                profile_image_url,
            },
        )
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))??
    .ok_or_else(|| {
        NoaError::AndroidUnavailable(
            "KakaoTalk 후킹 에이전트가 입장 결과를 반환하지 않았습니다".to_string(),
        )
    })?;
    let result: OpenChatJoinResult = serde_json::from_str(&value).map_err(|error| {
        NoaError::AndroidUnavailable(format!(
            "KakaoTalk 후킹 입장 결과가 잘못되었습니다: {error}"
        ))
    })?;
    if result.room_name.trim().is_empty() {
        return Err(NoaError::AndroidUnavailable(
            "KakaoTalk 후킹 입장 결과에 방 이름이 없습니다".to_string(),
        ));
    }
    Ok(result)
}

fn send_command_blocking(room_id: i64, action: KakaoAction) -> Result<Option<String>, NoaError> {
    if !super::kakao_active() {
        return Err(NoaError::AndroidUnavailable(
            "KakaoTalk 후킹 에이전트가 준비되지 않았습니다".to_string(),
        ));
    }
    let sender = COMMAND_SENDER.get().ok_or_else(|| {
        NoaError::AndroidUnavailable("네이티브 명령 채널이 준비되지 않았습니다".to_string())
    })?;
    let action_label = action.label();
    let action_timeout = action.timeout();
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
    match response_receiver.recv_timeout(action_timeout) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(message)) => Err(NoaError::AndroidUnavailable(message)),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            remove_pending(id);
            Err(NoaError::AndroidUnavailable(format!(
                "KakaoTalk 후킹 {action_label} 호출 시간이 초과되었습니다"
            )))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            remove_pending(id);
            Err(NoaError::AndroidUnavailable(format!(
                "KakaoTalk 후킹 {action_label} 응답 채널이 종료되었습니다"
            )))
        }
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
            config.iris_hook.endpoint_bridge_url.clone(),
            config.iris_hook.endpoint_prefix.clone(),
        );

        frida_init();
        frida_selinux_patch_policy();
        info!("Frida Android SELinux 정책 초기화 호출 완료");
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
        let mut iris_observed = None;
        let mut next_scan = Instant::now();
        let mut iris_retry = NativeInjectionRetry::new();
        let mut kakao_retry = NativeInjectionRetry::new();
        loop {
            pump();
            let now = Instant::now();
            if now >= next_scan {
                let discovered = config.iris_hook.enabled.then(iris_process_pid).flatten();
                iris_target = stable_process_target(&mut iris_observed, discovered);
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

fn stable_process_target(
    observed: &mut Option<(u32, Instant)>,
    discovered: Option<u32>,
) -> Option<u32> {
    let Some(pid) = discovered else {
        *observed = None;
        return None;
    };
    match observed {
        Some((observed_pid, since)) if *observed_pid == pid => {
            (since.elapsed() >= Duration::from_secs(2)).then_some(pid)
        }
        _ => {
            *observed = Some((pid, Instant::now()));
            None
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
    retry: &mut NativeInjectionRetry,
) {
    retry.observe_target(target);
    let changed = slot
        .as_ref()
        .is_some_and(|injection| target != Some(injection.pid));
    if (changed || target.is_none()) && slot.take().is_some() {
        kind.set_active(false);
        if matches!(kind, NativeKind::Kakao) {
            fail_pending("KakaoTalk 네이티브 후킹 연결이 종료되었습니다");
        }
    }
    if slot.is_some() && kind.active() {
        retry.record_success();
    }
    let stalled = slot.as_ref().is_some_and(|injection| {
        !kind.active() && injection.injected_at.elapsed() >= Duration::from_secs(15)
    });
    if stalled {
        *slot = None;
        let (consecutive_failures, retry_delay) = retry.record_failure();
        warn!(
            process = kind.label(),
            consecutive_failures,
            retry_after_seconds = retry_delay.as_secs(),
            "Rust 에이전트 준비 시간 초과; 재주입을 예약합니다"
        );
    }
    if slot.is_some() || target.is_none() || !retry.ready() {
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
            let (consecutive_failures, retry_delay) = retry.record_failure();
            warn!(
                pid,
                process = kind.label(),
                error = %message,
                consecutive_failures,
                retry_after_seconds = retry_delay.as_secs(),
                "Rust 에이전트 주입 실패"
            );
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
                "endpoint_prefix": config.iris_hook.endpoint_prefix,
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
        KakaoAction::LoadOpenChatMember { user_id } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "load-open-chat-member",
            "room": command.room_id,
            "user": user_id,
        }),
        KakaoAction::ShareOpenProfile => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "share-open-profile",
            "link": command.room_id,
        }),
        KakaoAction::JoinOpenChat {
            url,
            profile_id,
            profile_kind,
            nickname,
            profile_image_url,
        } => serde_json::json!({
            "token": token,
            "id": command.id,
            "action": "join-open-chat",
            "url": url,
            "profileId": profile_id,
            "profileKind": profile_kind,
            "nickname": nickname,
            "profileImageUrl": profile_image_url,
        }),
    };
    connection
        .stream
        .set_read_timeout(Some(
            command
                .action
                .timeout()
                .saturating_sub(Duration::from_secs(1)),
        ))
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
        Ok(response
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_string))
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
        if value.get("token").and_then(Value::as_str) != Some(token) {
            continue;
        }
        if value.get("event").and_then(Value::as_str) == Some("database-invalidated") {
            let Some(database) = value.get("database").and_then(Value::as_str) else {
                continue;
            };
            let Some(table) = value.get("table").and_then(Value::as_str) else {
                continue;
            };
            record_database_invalidation(DatabaseInvalidation {
                database: database.to_string(),
                table: table.to_string(),
                captured_at: value
                    .get("capturedAt")
                    .and_then(Value::as_i64)
                    .unwrap_or_default(),
            });
            continue;
        }
        if value.get("event").and_then(Value::as_str) != Some("loco") {
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

fn launch_iris_bridge(
    listener: TcpListener,
    token: String,
    bridge_url: String,
    endpoint_bridge_url: String,
    endpoint_prefix: String,
) {
    if let Err(error) = thread::Builder::new()
        .name("noa-iris-bridge".to_string())
        .spawn(move || {
            iris_bridge_loop(
                listener,
                token,
                bridge_url,
                endpoint_bridge_url,
                endpoint_prefix,
            )
        })
    {
        error!(%error, "Iris 네이티브 브리지 스레드를 시작하지 못했습니다");
    }
}

fn iris_bridge_loop(
    listener: TcpListener,
    token: String,
    bridge_url: String,
    endpoint_bridge_url: String,
    endpoint_prefix: String,
) {
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let token = token.clone();
                let bridge_url = bridge_url.clone();
                let endpoint_bridge_url = endpoint_bridge_url.clone();
                let endpoint_prefix = endpoint_prefix.clone();
                let _ = thread::Builder::new()
                    .name("noa-iris-request".to_string())
                    .spawn(move || {
                        if let Err(message) = handle_iris_connection(
                            stream,
                            &token,
                            &bridge_url,
                            &endpoint_bridge_url,
                            &endpoint_prefix,
                        ) {
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
    endpoint_bridge_url: &str,
    endpoint_prefix: &str,
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
        Some("endpoint") => {
            let method = request
                .get("method")
                .and_then(Value::as_str)
                .ok_or_else(|| "Iris endpoint method가 없습니다".to_string())?;
            let uri = request
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| "Iris endpoint URI가 없습니다".to_string())?;
            let content_type = request
                .get("contentType")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream");
            let body = request
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let result = forward_iris_endpoint_http(
                endpoint_bridge_url,
                endpoint_prefix,
                token,
                method,
                uri,
                content_type,
                body,
            );
            write_iris_endpoint_response(&mut stream, id, result)?;
        }
        _ => write_iris_response(&mut stream, id, Err("unknown Iris event".to_string()))?,
    }
    Ok(())
}

fn write_iris_endpoint_response(
    stream: &mut TcpStream,
    id: Option<u64>,
    result: Result<BridgeHttpResponse, String>,
) -> Result<(), String> {
    let response = match result {
        Ok(response) => serde_json::json!({
            "id": id,
            "ok": true,
            "status": response.status,
            "contentType": response.content_type,
            "body": response.body,
        }),
        Err(message) => serde_json::json!({"id": id, "ok": false, "error": message}),
    };
    serde_json::to_writer(&mut *stream, &response).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
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
    let response = iris_http_transaction(
        url,
        token,
        "POST",
        "application/json; charset=utf-8",
        payload,
    )?;
    if (200..300).contains(&response.status) {
        Ok(())
    } else {
        Err(format!(
            "Noa bridge returned HTTP {}{}",
            response.status,
            if response.body.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", response.body.trim())
            }
        ))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct BridgeHttpResponse {
    status: u16,
    content_type: String,
    body: String,
}

fn forward_iris_endpoint_http(
    bridge_url: &str,
    prefix: &str,
    token: &str,
    method: &str,
    uri: &str,
    content_type: &str,
    body: &str,
) -> Result<BridgeHttpResponse, String> {
    if !matches!(
        method,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" | "HEAD"
    ) {
        return Err(format!(
            "지원하지 않는 Iris endpoint method입니다: {method}"
        ));
    }
    let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
    let suffix = path
        .strip_prefix(prefix)
        .filter(|suffix| suffix.is_empty() || suffix.starts_with('/'))
        .ok_or_else(|| "Iris endpoint URI가 설정된 prefix 밖에 있습니다".to_string())?;
    let lower_path = path.to_ascii_lowercase();
    if path.split('/').any(|segment| matches!(segment, "." | "..")) || lower_path.contains("%2e") {
        return Err("Iris endpoint URI에 허용되지 않는 경로가 있습니다".to_string());
    }
    let mut target = url::Url::parse(bridge_url).map_err(|error| error.to_string())?;
    let base_path = target.path().trim_end_matches('/').to_string();
    target.set_path(&format!(
        "{base_path}{}",
        if suffix.is_empty() { "/" } else { suffix }
    ));
    target.set_query((!query.is_empty()).then_some(query));
    iris_http_transaction(target.as_str(), token, method, content_type, body)
}

fn iris_http_transaction(
    url: &str,
    token: &str,
    method: &str,
    content_type: &str,
    payload: &str,
) -> Result<BridgeHttpResponse, String> {
    if [token, content_type]
        .into_iter()
        .any(|value| value.bytes().any(|byte| byte < b' ' || byte == 0x7f))
    {
        return Err("Iris 내부 브리지 HTTP 헤더 값이 올바르지 않습니다".to_string());
    }
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
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: {content_type}\r\nX-Noa-Hook-Token: {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .map_err(|error| error.to_string())?;
    stream
        .write_all(payload.as_bytes())
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut response = Vec::new();
    stream
        .take(MAX_IRIS_HTTP_RESPONSE_BYTES + 1)
        .read_to_end(&mut response)
        .map_err(|error| error.to_string())?;
    if response.len() as u64 > MAX_IRIS_HTTP_RESPONSE_BYTES {
        return Err("Noa endpoint 응답이 허용 크기를 초과했습니다".to_string());
    }
    parse_iris_http_response(&response)
}

fn parse_iris_http_response(response: &[u8]) -> Result<BridgeHttpResponse, String> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "잘못된 HTTP 응답 헤더".to_string())?;
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let mut lines = headers.lines();
    let status_line = lines.next().ok_or_else(|| "빈 HTTP 응답".to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| format!("잘못된 HTTP 상태행: {status_line}"))?;
    let fields = lines
        .filter_map(|line| line.split_once(':'))
        .collect::<Vec<_>>();
    let content_type = fields
        .iter()
        .find_map(|(name, value)| {
            name.eq_ignore_ascii_case("content-type")
                .then(|| value.trim().to_string())
        })
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let raw_body = &response[header_end + 4..];
    let body = if fields.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
    }) {
        decode_chunked_body(raw_body)?
    } else {
        raw_body.to_vec()
    };
    let body = String::from_utf8(body)
        .map_err(|_| "Noa endpoint 응답 본문은 UTF-8이어야 합니다".to_string())?;
    Ok(BridgeHttpResponse {
        status,
        content_type,
        body,
    })
}

fn decode_chunked_body(mut encoded: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    loop {
        let line_end = encoded
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| "잘못된 chunked HTTP 응답".to_string())?;
        let size_text = std::str::from_utf8(&encoded[..line_end])
            .map_err(|_| "잘못된 chunk 크기".to_string())?
            .split(';')
            .next()
            .unwrap_or_default()
            .trim();
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| "잘못된 chunk 크기".to_string())?;
        encoded = &encoded[line_end + 2..];
        if size == 0 {
            return Ok(decoded);
        }
        if encoded.len() < size + 2 || &encoded[size..size + 2] != b"\r\n" {
            return Err("완전하지 않은 chunked HTTP 응답".to_string());
        }
        decoded.extend_from_slice(&encoded[..size]);
        encoded = &encoded[size + 2..];
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

fn complete_pending(id: u64, result: Result<Option<String>, String>) {
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
