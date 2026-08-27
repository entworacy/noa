use std::{
    collections::VecDeque,
    sync::{
        Arc, OnceLock, RwLock,
        atomic::{AtomicBool, Ordering},
    },
};

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
