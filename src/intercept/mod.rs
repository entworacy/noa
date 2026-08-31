use std::{
    collections::VecDeque,
    sync::{
        Arc, OnceLock, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[cfg(any(target_os = "android", test))]
use std::time::{Duration, Instant};

use crate::{model::LocoPacket, settings::Settings};
use tokio::sync::broadcast;

#[cfg(target_os = "android")]
mod runtime;
#[cfg(target_os = "android")]
pub use runtime::OpenChatJoinResult;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static KAKAO_ACTIVE: AtomicBool = AtomicBool::new(false);
static LOCO_PACKETS: OnceLock<RwLock<VecDeque<LocoPacket>>> = OnceLock::new();
static DATABASE_INVALIDATIONS: OnceLock<broadcast::Sender<DatabaseInvalidation>> = OnceLock::new();
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
static LOCO_LIMIT: OnceLock<usize> = OnceLock::new();

#[cfg(any(target_os = "android", test))]
struct NativeInjectionRetry {
    target: Option<u32>,
    consecutive_failures: u32,
    retry_at: Instant,
}

#[cfg(any(target_os = "android", test))]
impl NativeInjectionRetry {
    fn new() -> Self {
        Self {
            target: None,
            consecutive_failures: 0,
            retry_at: Instant::now(),
        }
    }

    fn observe_target(&mut self, target: Option<u32>) {
        if self.target != target {
            self.target = target;
            self.consecutive_failures = 0;
            self.retry_at = Instant::now();
        }
    }

    fn ready(&self) -> bool {
        Instant::now() >= self.retry_at
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.retry_at = Instant::now();
    }

    fn record_failure(&mut self) -> (u32, Duration) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let delay = native_injection_retry_delay(self.consecutive_failures);
        self.retry_at = Instant::now() + delay;
        (self.consecutive_failures, delay)
    }
}

#[cfg(any(target_os = "android", test))]
fn native_injection_retry_delay(consecutive_failures: u32) -> Duration {
    const DELAYS_SECONDS: [u64; 6] = [2, 4, 8, 16, 32, 60];
    let index = consecutive_failures
        .saturating_sub(1)
        .min((DELAYS_SECONDS.len() - 1) as u32) as usize;
    Duration::from_secs(DELAYS_SECONDS[index])
}

#[derive(Clone, Debug)]
pub struct DatabaseInvalidation {
    pub database: String,
    pub table: String,
    pub captured_at: i64,
}

pub fn subscribe_database_invalidations() -> broadcast::Receiver<DatabaseInvalidation> {
    database_invalidation_sender().subscribe()
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn record_database_invalidation(invalidation: DatabaseInvalidation) {
    let _ = database_invalidation_sender().send(invalidation);
}

fn database_invalidation_sender() -> &'static broadcast::Sender<DatabaseInvalidation> {
    DATABASE_INVALIDATIONS.get_or_init(|| broadcast::channel(128).0)
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

pub fn kakao_active() -> bool {
    KAKAO_ACTIVE.load(Ordering::Acquire)
}

#[cfg(target_os = "android")]
pub fn launch(config: Arc<Settings>) {
    if !config.iris_hook.enabled && !config.kakao_hook_enabled {
        return;
    }
    let _ = LOCO_LIMIT.set(config.loco_history_limit);
    runtime::launch(config);
}

#[cfg(not(target_os = "android"))]
pub fn launch(_: Arc<Settings>) {}

pub fn loco_packets(limit: usize) -> Vec<LocoPacket> {
    let packets = LOCO_PACKETS.get_or_init(|| RwLock::new(VecDeque::new()));
    packets
        .read()
        .map(|values| {
            values
                .iter()
                .rev()
                .take(limit.clamp(1, 10_000))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn record_loco_packet(packet: LocoPacket) {
    let packets = LOCO_PACKETS.get_or_init(|| RwLock::new(VecDeque::new()));
    if let Ok(mut values) = packets.write() {
        let limit = *LOCO_LIMIT.get().unwrap_or(&1_000);
        while values.len() >= limit {
            values.pop_front();
        }
        values.push_back(packet);
    }
}

#[cfg(target_os = "android")]
pub async fn send_custom(room_id: i64, row_id: i64) -> Result<(), crate::failure::NoaError> {
    runtime::send_custom(room_id, row_id).await
}

#[cfg(not(target_os = "android"))]
pub async fn send_custom(_: i64, _: i64) -> Result<(), crate::failure::NoaError> {
    Err(crate::failure::NoaError::AndroidUnavailable(
        "KakaoTalk 후킹 발신은 Android에서만 사용할 수 있습니다".to_string(),
    ))
}

#[cfg(target_os = "android")]
pub async fn kick_member(room_id: i64, user_id: i64) -> Result<(), crate::failure::NoaError> {
    runtime::kick_member(room_id, user_id).await
}

#[cfg(not(target_os = "android"))]
#[allow(dead_code)]
pub async fn kick_member(_: i64, _: i64) -> Result<(), crate::failure::NoaError> {
    Err(crate::failure::NoaError::AndroidUnavailable(
        "KakaoTalk 후킹 강퇴는 Android에서만 사용할 수 있습니다".to_string(),
    ))
}

#[cfg(target_os = "android")]
pub async fn chat_on_room(room_id: i64) -> Result<(), crate::failure::NoaError> {
    runtime::chat_on_room(room_id).await
}

#[cfg(target_os = "android")]
pub async fn load_open_chat_member(
    room_id: i64,
    user_id: i64,
) -> Result<(), crate::failure::NoaError> {
    runtime::load_open_chat_member(room_id, user_id).await
}

#[cfg(target_os = "android")]
pub async fn share_open_profile(link_id: i64) -> Result<String, crate::failure::NoaError> {
    runtime::share_open_profile(link_id).await
}

#[cfg(target_os = "android")]
pub async fn join_open_chat(
    url: String,
    profile_id: String,
    profile_kind: String,
    nickname: String,
    profile_image_url: Option<String>,
) -> Result<OpenChatJoinResult, crate::failure::NoaError> {
    runtime::join_open_chat(url, profile_id, profile_kind, nickname, profile_image_url).await
}

pub async fn vox_start_call(
    room_id: i64,
    caller_id: i64,
    peer_ids: Vec<i64>,
    open_chat: bool,
    team_chat: bool,
    group_chat: bool,
) -> Result<(), crate::failure::NoaError> {
    #[cfg(target_os = "android")]
    {
        runtime::vox_start_call(
            room_id, caller_id, peer_ids, open_chat, team_chat, group_chat,
        )
        .await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (
            room_id, caller_id, peer_ids, open_chat, team_chat, group_chat,
        );
        Err(vox_unavailable())
    }
}

pub async fn vox_create_room(room_id: i64, title: String) -> Result<(), crate::failure::NoaError> {
    #[cfg(target_os = "android")]
    {
        runtime::vox_create_room(room_id, title).await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (room_id, title);
        Err(vox_unavailable())
    }
}

pub async fn vox_join_room(
    room_id: i64,
    call_id: i64,
    host_v4: String,
    host_v6: String,
    port: i32,
) -> Result<(), crate::failure::NoaError> {
    #[cfg(target_os = "android")]
    {
        runtime::vox_join_room(room_id, call_id, host_v4, host_v6, port).await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (room_id, call_id, host_v4, host_v6, port);
        Err(vox_unavailable())
    }
}

pub async fn vox_leave(room_id: i64, kind: String) -> Result<(), crate::failure::NoaError> {
    #[cfg(target_os = "android")]
    {
        runtime::vox_leave(room_id, kind).await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (room_id, kind);
        Err(vox_unavailable())
    }
}

pub async fn vox_status() -> Result<serde_json::Value, crate::failure::NoaError> {
    #[cfg(target_os = "android")]
    {
        runtime::vox_status().await
    }
    #[cfg(not(target_os = "android"))]
    {
        Err(vox_unavailable())
    }
}

pub async fn vox_audio_start(mode: String) -> Result<serde_json::Value, crate::failure::NoaError> {
    #[cfg(target_os = "android")]
    {
        runtime::vox_audio_start(mode).await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = mode;
        Err(vox_unavailable())
    }
}

pub async fn vox_audio_push(bytes: Vec<u8>) -> Result<serde_json::Value, crate::failure::NoaError> {
    #[cfg(target_os = "android")]
    {
        runtime::vox_audio_push(bytes).await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = bytes;
        Err(vox_unavailable())
    }
}

pub async fn vox_audio_stop() -> Result<serde_json::Value, crate::failure::NoaError> {
    #[cfg(target_os = "android")]
    {
        runtime::vox_audio_stop().await
    }
    #[cfg(not(target_os = "android"))]
    {
        Err(vox_unavailable())
    }
}

#[cfg(not(target_os = "android"))]
fn vox_unavailable() -> crate::failure::NoaError {
    crate::failure::NoaError::AndroidUnavailable(
        "VOX 후킹은 Android에서만 사용할 수 있습니다".to_string(),
    )
}

#[cfg(not(target_os = "android"))]
#[allow(dead_code)]
pub async fn share_open_profile(_: i64) -> Result<String, crate::failure::NoaError> {
    Err(crate::failure::NoaError::AndroidUnavailable(
        "오픈프로필 후킹 공유는 Android에서만 사용할 수 있습니다".to_string(),
    ))
}

#[cfg(not(target_os = "android"))]
pub async fn chat_on_room(_: i64) -> Result<(), crate::failure::NoaError> {
    Err(crate::failure::NoaError::AndroidUnavailable(
        "CHATONROOM 내부 함수 호출은 Android에서만 사용할 수 있습니다".to_string(),
    ))
}

#[cfg(not(target_os = "android"))]
pub async fn load_open_chat_member(_: i64, _: i64) -> Result<(), crate::failure::NoaError> {
    Err(crate::failure::NoaError::AndroidUnavailable(
        "오픈채팅 멤버 프로필 조회는 Android에서만 사용할 수 있습니다".to_string(),
    ))
}

pub fn launch_chatonroom_rotation(
    rooms: Arc<tokio::sync::RwLock<Vec<crate::model::Room>>>,
    config: Arc<Settings>,
) {
    if !config.kakao_hook_enabled {
        if config.chatonroom_interval_ms > 0 {
            tracing::info!(
                "접근성 모드에서는 화면을 계속 점유하지 않도록 선택적 CHATONROOM 순회를 비활성화합니다"
            );
        }
        return;
    }
    if config.chatonroom_interval_ms == 0 {
        return;
    }
    tokio::spawn(async move {
        let mut index = 0usize;
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(
            config.chatonroom_interval_ms.max(1_000),
        ));
        interval.tick().await;
        loop {
            interval.tick().await;
            if !kakao_active() {
                continue;
            }
            let ids: Vec<i64> = rooms
                .read()
                .await
                .iter()
                .filter_map(|room| room.chat_id.parse::<i64>().ok())
                .filter(|id| id & (1_i64 << 54) != 0)
                .collect();
            if ids.is_empty() {
                continue;
            }
            let room = ids[index % ids.len()];
            index = (index + 1) % ids.len();
            if let Err(error) = chat_on_room(room).await {
                tracing::warn!(room, %error, "CHATONROOM 순회 호출 실패");
            }
        }
    });
}

#[cfg(target_os = "android")]
fn set_active(value: bool) {
    ACTIVE.store(value, Ordering::Release);
}

#[cfg(target_os = "android")]
fn set_kakao_active(value: bool) {
    KAKAO_ACTIVE.store(value, Ordering::Release);
}

#[cfg(test)]
mod tests {
    use super::{NativeInjectionRetry, native_injection_retry_delay};
    use std::time::Duration;

    #[test]
    fn native_injection_retry_uses_capped_exponential_delays() {
        let expected = [2, 4, 8, 16, 32, 60, 60, 60];
        for (failure, seconds) in (1_u32..).zip(expected) {
            assert_eq!(
                native_injection_retry_delay(failure),
                Duration::from_secs(seconds)
            );
        }
    }

    #[test]
    fn native_injection_retry_resets_for_a_new_process() {
        let mut retry = NativeInjectionRetry::new();
        retry.observe_target(Some(10));
        retry.record_failure();
        assert_eq!(retry.consecutive_failures, 1);
        assert!(!retry.ready());

        retry.observe_target(Some(11));
        assert_eq!(retry.consecutive_failures, 0);
        assert!(retry.ready());
    }
}
