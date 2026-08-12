use crate::{asset::PreparedAsset, failure::NoaError, settings::Settings};

#[cfg(target_os = "android")]
use std::sync::Arc;

#[cfg(target_os = "android")]
use tokio::sync::Mutex;

#[cfg(any(target_os = "android", test))]
mod accessibility;
#[cfg(any(target_os = "android", test))]
mod envelope;
#[cfg(any(target_os = "android", test))]
mod framework;
#[cfg(any(target_os = "android", test))]
mod queue;
#[cfg(any(target_os = "android", test))]
mod stubs;
#[cfg(target_os = "android")]
mod ui_agent;
#[cfg(any(target_os = "android", test))]
mod vm;

#[cfg(target_os = "android")]
#[derive(Clone)]
pub struct KakaoRelay(queue::OutboundQueue, i32, Arc<Mutex<()>>);

#[cfg(not(target_os = "android"))]
#[derive(Clone)]
pub struct KakaoRelay;

impl KakaoRelay {
    #[cfg(target_os = "android")]
    pub fn connect(config: &Settings) -> Result<Self, NoaError> {
        queue::OutboundQueue::connect(config)
            .map(|queue| Self(queue, config.android_user_id, Arc::new(Mutex::new(()))))
    }

    #[cfg(not(target_os = "android"))]
    pub fn connect(_: &Settings) -> Result<Self, NoaError> {
        Err(NoaError::AndroidUnavailable(
            "이 바이너리는 Android 대상으로 빌드되지 않았습니다".to_string(),
        ))
    }

    #[cfg(target_os = "android")]
    pub async fn deliver_asset(&self, room_id: i64, asset: PreparedAsset) -> Result<(), NoaError> {
        self.0.deliver_files(room_id, vec![asset]).await
    }

    #[cfg(not(target_os = "android"))]
    pub async fn deliver_asset(&self, _: i64, _: PreparedAsset) -> Result<(), NoaError> {
        unavailable()
    }

    #[cfg(target_os = "android")]
    pub async fn deliver_text(
        &self,
        room_id: i64,
        text: String,
        thread_id: Option<i64>,
    ) -> Result<(), NoaError> {
        self.0.deliver_text(room_id, text, thread_id).await
    }

    #[cfg(not(target_os = "android"))]
    pub async fn deliver_text(&self, _: i64, _: String, _: Option<i64>) -> Result<(), NoaError> {
        unavailable()
    }

    #[cfg(target_os = "android")]
    pub async fn deliver_markdown(&self, room_id: i64, text: String) -> Result<(), NoaError> {
        self.0.deliver_markdown(room_id, text).await
    }

    #[cfg(not(target_os = "android"))]
    pub async fn deliver_markdown(&self, _: i64, _: String) -> Result<(), NoaError> {
        unavailable()
    }

    #[cfg(target_os = "android")]
    pub async fn resend_custom(
        &self,
        room_id: i64,
        message: String,
        attachment: String,
    ) -> Result<(), NoaError> {
        let _guard = self.2.lock().await;
        let profile = self.1;
        tokio::task::spawn_blocking(move || {
            accessibility::resend(room_id, profile, &message, &attachment)
        })
        .await
        .map_err(|error| NoaError::AndroidUnavailable(error.to_string()))?
    }

    #[cfg(not(target_os = "android"))]
    pub async fn resend_custom(&self, _: i64, _: String, _: String) -> Result<(), NoaError> {
        unavailable()
    }

    #[cfg(target_os = "android")]
    pub async fn join_open_chat(
        &self,
        url: String,
        requested_profile: Option<String>,
    ) -> Result<(String, Option<String>), NoaError> {
        let _guard = self.2.lock().await;
        let profile = self.1;
        tokio::task::spawn_blocking(move || {
            accessibility::join_open_chat(&url, profile, requested_profile.as_deref())
        })
        .await
        .map_err(|error| NoaError::AndroidUnavailable(error.to_string()))?
    }

    #[cfg(not(target_os = "android"))]
    pub async fn join_open_chat(
        &self,
        _: String,
        _: Option<String>,
    ) -> Result<(String, Option<String>), NoaError> {
        unavailable()
    }

    #[cfg(target_os = "android")]
    pub async fn leave_chat(&self, room_id: i64, room_name: String) -> Result<(), NoaError> {
        let _guard = self.2.lock().await;
        let profile = self.1;
        tokio::task::spawn_blocking(move || accessibility::leave_chat(room_id, profile, &room_name))
            .await
            .map_err(|error| NoaError::AndroidUnavailable(error.to_string()))?
    }

    #[cfg(not(target_os = "android"))]
    pub async fn leave_chat(&self, _: i64, _: String) -> Result<(), NoaError> {
        unavailable()
    }

    #[cfg(target_os = "android")]
    pub async fn kick_member(
        &self,
        room_id: i64,
        room_name: String,
        nickname: String,
        user_id: i64,
    ) -> Result<(), NoaError> {
        if crate::intercept::kakao_active() {
            return crate::intercept::kick_member(room_id, user_id).await;
        }
        let _guard = self.2.lock().await;
        let profile = self.1;
        tokio::task::spawn_blocking(move || {
            accessibility::kick_member(room_id, profile, &room_name, &nickname)
        })
        .await
        .map_err(|error| NoaError::AndroidUnavailable(error.to_string()))?
    }

    #[cfg(not(target_os = "android"))]
    pub async fn kick_member(&self, _: i64, _: String, _: String, _: i64) -> Result<(), NoaError> {
        unavailable()
    }
}

#[cfg(not(target_os = "android"))]
fn unavailable<T>() -> Result<T, NoaError> {
    Err(NoaError::AndroidUnavailable(
        "호스트 빌드에서는 KakaoTalk 전송을 실행할 수 없습니다".to_string(),
    ))
}
