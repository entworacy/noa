use super::{
    commands::{CHANNELS, ChannelState, fail_pending, transport::launch_kakao_bridge},
    events::launch_kakao_event_bridge,
    frida::*,
    iris_bridge::launch_iris_bridge,
    process::{
        iris_process_pid, kakao_process_pid, log_kakao_process_discovery, stable_process_target,
    },
    state::{IRIS_FAILED_PID, KAKAO_FATAL_PID, KAKAO_TARGET_PID},
};
use crate::{
    intercept::{self, NativeInjectionRetry, set_active, set_kakao_active},
    settings::Settings,
};
use noa_agent_protocol::Channel;
use std::{
    ffi::CString,
    net::{TcpListener, TcpStream, ToSocketAddrs},
    ptr,
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};
use tracing::{error, info, warn};
const KAKAO_AGENT: &[u8] = include_bytes!(env!("NOA_KAKAO_AGENT_BLOB"));
const IRIS_AGENT: &[u8] = include_bytes!(env!("NOA_IRIS_AGENT_BLOB"));
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
            Self::Iris => intercept::active(),
            Self::Kakao => intercept::kakao_active(),
        }
    }

    fn set_active(self, value: bool) {
        match self {
            Self::Iris => set_active(value),
            Self::Kakao => set_kakao_active(value),
        }
    }

    fn readiness_timeout(self) -> Duration {
        match self {
            Self::Iris => Duration::from_secs(15),
            Self::Kakao => Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Copy)]
struct AgentPorts {
    command: u16,
    event: u16,
    vox: u16,
    audio: u16,
}

struct NativeInjection {
    pid: u32,
    injected_at: Instant,
}
pub fn launch(config: Arc<Settings>) {
    if let Err(error) = thread::Builder::new()
        .name("noa-intercept".to_string())
        .spawn(move || run(config))
    {
        error!(%error, "네이티브 후킹 스레드를 시작하지 못했습니다");
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
        let vox_listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(v) => v,
            Err(e) => {
                error!(%e, "VOX 채널을 열지 못했습니다");
                return;
            }
        };
        let audio_listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(v) => v,
            Err(e) => {
                error!(%e, "오디오 채널을 열지 못했습니다");
                return;
            }
        };
        let ports = AgentPorts {
            command: kakao_port,
            event: kakao_event_port,
            vox: vox_listener.local_addr().unwrap().port(),
            audio: audio_listener.local_addr().unwrap().port(),
        };
        let mut states = Vec::new();
        for (channel, listener) in
            Channel::ALL
                .into_iter()
                .zip([kakao_listener, vox_listener, audio_listener])
        {
            let (state, receiver) =
                ChannelState::new(if channel == Channel::Audio { 8 } else { 32 });
            let state = Arc::new(state);
            if let Err(e) = launch_kakao_bridge(
                listener,
                receiver,
                config.iris_hook.token.clone(),
                channel,
                state.clone(),
            ) {
                error!(%e, "에이전트 채널 시작 실패");
                return;
            }
            states.push(state);
        }
        if CHANNELS
            .set(states.try_into().unwrap_or_else(|_| unreachable!()))
            .is_err()
        {
            error!("명령 채널이 이미 시작되었습니다");
            return;
        }
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
        let mut kakao_observed = None;
        let mut kakao_discovery_observed = false;
        let mut last_kakao_discovered = None;
        let mut next_scan = Instant::now();
        let mut iris_retry = NativeInjectionRetry::new();
        let mut kakao_retry = NativeInjectionRetry::new();
        loop {
            pump();
            let now = Instant::now();
            if now >= next_scan {
                let discovered = config
                    .iris_hook
                    .enabled
                    .then(iris_process_pid)
                    .flatten()
                    .filter(|_| iris_service_ready());
                iris_target = stable_process_target(&mut iris_observed, discovered);
                let discovered = config.kakao_hook_enabled.then(kakao_process_pid).flatten();
                if config.kakao_hook_enabled
                    && (!kakao_discovery_observed || discovered != last_kakao_discovered)
                {
                    log_kakao_process_discovery(discovered);
                    kakao_discovery_observed = true;
                    last_kakao_discovered = discovered;
                }
                kakao_target = stable_process_target(&mut kakao_observed, discovered);
                let target_pid = kakao_target.unwrap_or(0);
                if KAKAO_TARGET_PID.swap(target_pid, Ordering::AcqRel) != target_pid {
                    set_kakao_active(false);
                    fail_pending("KakaoTalk 대상 프로세스가 변경되었습니다");
                }
                next_scan = now + Duration::from_millis(500);
            }
            refresh_native_agent(
                device,
                &mut iris_injection,
                iris_target,
                NativeKind::Iris,
                AgentPorts {
                    command: iris_port,
                    ..ports
                },
                &config,
                &mut iris_retry,
            );
            refresh_native_agent(
                device,
                &mut kakao_injection,
                kakao_target,
                NativeKind::Kakao,
                ports,
                &config,
                &mut kakao_retry,
            );
            pump();
            thread::sleep(Duration::from_millis(25));
        }
    }
}

fn iris_service_ready() -> bool {
    "127.0.0.1:3000"
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next())
        .is_some_and(|address| {
            TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok()
        })
}
unsafe fn refresh_native_agent(
    device: *mut FridaDevice,
    slot: &mut Option<NativeInjection>,
    target: Option<u32>,
    kind: NativeKind,
    ports: AgentPorts,
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
    if matches!(kind, NativeKind::Kakao) {
        let fatal_pid = KAKAO_FATAL_PID.load(Ordering::Acquire);
        if fatal_pid != 0 && target != Some(fatal_pid) {
            KAKAO_FATAL_PID.store(0, Ordering::Release);
        } else if target == Some(fatal_pid) {
            return;
        }
    }
    if matches!(kind, NativeKind::Iris) {
        let failed_pid = IRIS_FAILED_PID.load(Ordering::Acquire);
        if failed_pid != 0 && target != Some(failed_pid) {
            IRIS_FAILED_PID.store(0, Ordering::Release);
        } else if failed_pid != 0
            && slot
                .as_ref()
                .is_some_and(|injection| injection.pid == failed_pid)
        {
            *slot = None;
            IRIS_FAILED_PID.store(0, Ordering::Release);
            let (consecutive_failures, retry_delay) = retry.record_failure();
            warn!(
                pid = failed_pid,
                process = kind.label(),
                consecutive_failures,
                retry_after_seconds = retry_delay.as_secs(),
                "Rust 에이전트가 초기화 실패를 보고하여 재주입을 예약합니다"
            );
        }
    }
    let stalled = slot.as_ref().is_some_and(|injection| {
        !kind.active() && injection.injected_at.elapsed() >= kind.readiness_timeout()
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
    match unsafe { inject_native_agent(device, pid, kind, ports, config) } {
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
    ports: AgentPorts,
    config: &Settings,
) -> Result<u32, String> {
    let (agent, entrypoint, data) = match kind {
        NativeKind::Iris => (
            IRIS_AGENT,
            c"noa_iris_agent_main",
            serde_json::json!({
                "port": ports.command,
                "token": config.iris_hook.token,
                "types": config.iris_hook.types,
                "endpoint_prefix": config.iris_hook.endpoint_prefix,
            }),
        ),
        NativeKind::Kakao => (
            KAKAO_AGENT,
            c"noa_agent_main",
            serde_json::json!({
                "port": ports.command,
                "event_port": ports.event,
                "vox_port": ports.vox,
                "audio_port": ports.audio,
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
