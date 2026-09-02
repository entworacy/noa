use crate::{asset::PreparedAsset, failure::NoaError, settings::Settings};

#[cfg(target_os = "android")]
use std::sync::Arc;

#[cfg(target_os = "android")]
use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

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
pub struct KakaoRelay {
    queue: queue::OutboundQueue,
    android_user_id: i32,
    kakao_hook_enabled: bool,
    accessibility_lock: Arc<Mutex<()>>,
}

#[cfg(not(target_os = "android"))]
#[derive(Clone)]
pub struct KakaoRelay;

impl KakaoRelay {
    #[cfg(target_os = "android")]
    pub fn connect(config: &Settings) -> Result<Self, NoaError> {
        verify_art_preflight()?;
        let queue = queue::OutboundQueue::connect(config)?;
        if let Err(error) = ui_agent::ensure_agent() {
            tracing::warn!(%error, "접근성 UI 에이전트 사전 준비 실패; 첫 접근성 요청에서 재시도합니다");
        }
        Ok(Self {
            queue,
            android_user_id: config.android_user_id,
            kakao_hook_enabled: config.kakao_hook_enabled,
            accessibility_lock: Arc::new(Mutex::new(())),
        })
    }

    #[cfg(not(target_os = "android"))]
    pub fn connect(_: &Settings) -> Result<Self, NoaError> {
        Err(NoaError::AndroidUnavailable(
            "이 바이너리는 Android 대상으로 빌드되지 않았습니다".to_string(),
        ))
    }

    #[cfg(target_os = "android")]
    pub async fn deliver_asset(&self, room_id: i64, asset: PreparedAsset) -> Result<(), NoaError> {
        self.queue.deliver_files(room_id, vec![asset]).await
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
        self.queue.deliver_text(room_id, text, thread_id).await
    }

    #[cfg(not(target_os = "android"))]
    pub async fn deliver_text(&self, _: i64, _: String, _: Option<i64>) -> Result<(), NoaError> {
        unavailable()
    }

    #[cfg(target_os = "android")]
    pub async fn deliver_markdown(&self, room_id: i64, text: String) -> Result<(), NoaError> {
        self.queue.deliver_markdown(room_id, text).await
    }

    #[cfg(not(target_os = "android"))]
    pub async fn deliver_markdown(&self, _: i64, _: String) -> Result<(), NoaError> {
        unavailable()
    }

    #[cfg(target_os = "android")]
    pub async fn resend_custom(
        &self,
        room_id: i64,
        room_name: String,
        message: String,
        attachment: String,
    ) -> Result<(), NoaError> {
        let _guard = self.accessibility_lock.lock().await;
        let profile = self.android_user_id;
        tokio::task::spawn_blocking(move || {
            accessibility::resend(room_id, profile, &room_name, &message, &attachment)
        })
        .await
        .map_err(|error| NoaError::AndroidUnavailable(error.to_string()))?
    }

    #[cfg(not(target_os = "android"))]
    pub async fn resend_custom(
        &self,
        _: i64,
        _: String,
        _: String,
        _: String,
    ) -> Result<(), NoaError> {
        unavailable()
    }

    #[cfg(target_os = "android")]
    pub async fn join_open_chat(
        &self,
        url: String,
        profile_id: String,
        profile_kind: String,
        nickname: String,
        profile_image_url: Option<String>,
    ) -> Result<(String, Option<String>), NoaError> {
        if self.kakao_hook_enabled {
            tracing::info!(
                profile_id,
                profile_kind,
                mode = "hook",
                "오픈채팅 입장 경로 선택"
            );
            let result = crate::intercept::join_open_chat(
                url,
                profile_id,
                profile_kind,
                nickname.clone(),
                profile_image_url,
            )
            .await?;
            return Ok((result.room_name, result.profile_applied.then_some(nickname)));
        }
        tracing::info!(
            profile_id,
            profile_kind,
            mode = "accessibility",
            "오픈채팅 입장 경로 선택"
        );
        let _guard = self.accessibility_lock.lock().await;
        let profile = self.android_user_id;
        tokio::task::spawn_blocking(move || accessibility::join_open_chat(&url, profile, &nickname))
            .await
            .map_err(|error| NoaError::AndroidUnavailable(error.to_string()))?
    }

    #[cfg(not(target_os = "android"))]
    pub async fn join_open_chat(
        &self,
        _: String,
        _: String,
        _: String,
        _: String,
        _: Option<String>,
    ) -> Result<(String, Option<String>), NoaError> {
        unavailable()
    }

    #[cfg(target_os = "android")]
    pub async fn share_open_profile(
        &self,
        link_id: i64,
        expected_url: Option<String>,
        use_hook: bool,
    ) -> Result<String, NoaError> {
        if use_hook {
            if !self.kakao_hook_enabled {
                return Err(NoaError::AndroidUnavailable(
                    "KakaoTalk 후킹이 비활성화되어 있습니다".to_string(),
                ));
            }
            tracing::info!(link_id, mode = "hook", "오픈프로필 공유 경로 선택");
            let url = crate::intercept::share_open_profile(link_id).await?;
            if expected_url
                .as_ref()
                .is_some_and(|expected| expected != &url)
            {
                return Err(NoaError::AndroidUnavailable(
                    "KakaoTalk 후킹 링크가 DB의 linkId 링크와 일치하지 않습니다".to_string(),
                ));
            }
            let _guard = self.accessibility_lock.lock().await;
            let profile = self.android_user_id;
            tokio::task::spawn_blocking(move || {
                accessibility::open_profile_activity(link_id, profile)
            })
            .await
            .map_err(|error| NoaError::AndroidUnavailable(error.to_string()))??;
            return Ok(url);
        }
        tracing::info!(link_id, mode = "accessibility", "오픈프로필 공유 경로 선택");
        let _guard = self.accessibility_lock.lock().await;
        let profile = self.android_user_id;
        tokio::task::spawn_blocking(move || {
            accessibility::share_open_profile(link_id, profile, expected_url.as_deref())
        })
        .await
        .map_err(|error| NoaError::AndroidUnavailable(error.to_string()))?
    }

    #[cfg(target_os = "android")]
    pub async fn share_member_open_profile(
        &self,
        room_id: i64,
        room_name: String,
        nickname: String,
    ) -> Result<String, NoaError> {
        tracing::info!(
            room_id,
            mode = "accessibility",
            "멤버 오픈프로필 공유 경로 선택"
        );
        let _guard = self.accessibility_lock.lock().await;
        let profile = self.android_user_id;
        tokio::task::spawn_blocking(move || {
            accessibility::share_member_open_profile(room_id, profile, &room_name, &nickname)
        })
        .await
        .map_err(|error| NoaError::AndroidUnavailable(error.to_string()))?
    }

    #[cfg(not(target_os = "android"))]
    pub async fn share_open_profile(
        &self,
        _: i64,
        _: Option<String>,
        _: bool,
    ) -> Result<String, NoaError> {
        unavailable()
    }

    #[cfg(not(target_os = "android"))]
    pub async fn share_member_open_profile(
        &self,
        _: i64,
        _: String,
        _: String,
    ) -> Result<String, NoaError> {
        unavailable()
    }

    #[cfg(target_os = "android")]
    pub async fn leave_chat(&self, room_id: i64, room_name: String) -> Result<(), NoaError> {
        let _guard = self.accessibility_lock.lock().await;
        let profile = self.android_user_id;
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
        if self.kakao_hook_enabled {
            tracing::info!(room_id, user_id, mode = "hook", "참여자 강퇴 경로 선택");
            return crate::intercept::kick_member(room_id, user_id).await;
        }
        tracing::info!(
            room_id,
            user_id,
            mode = "accessibility",
            "참여자 강퇴 경로 선택"
        );
        let _guard = self.accessibility_lock.lock().await;
        let profile = self.android_user_id;
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

#[cfg(target_os = "android")]
const ART_PREFLIGHT_ENV: &str = "NOA_INTERNAL_ART_PREFLIGHT";

#[cfg(target_os = "android")]
pub fn art_preflight_requested() -> bool {
    std::env::var_os(ART_PREFLIGHT_ENV).is_some()
}

#[cfg(not(target_os = "android"))]
pub fn art_preflight_requested() -> bool {
    false
}

#[cfg(target_os = "android")]
pub fn run_art_preflight() -> Result<(), String> {
    tracing::info!("별도 프로세스에서 Android ART 사전 검사를 실행합니다");
    let _vm = unsafe { vm::RuntimeVm::launch() }?;
    Ok(())
}

#[cfg(not(target_os = "android"))]
pub fn run_art_preflight() -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "android")]
fn verify_art_preflight() -> Result<(), NoaError> {
    let executable = std::env::current_exe().map_err(|error| {
        NoaError::AndroidUnavailable(format!("ART 사전 검사 실행 파일 확인 실패: {error}"))
    })?;
    let mut child = Command::new(executable)
        .env(ART_PREFLIGHT_ENV, "1")
        .spawn()
        .map_err(|error| {
            NoaError::AndroidUnavailable(format!("ART 사전 검사 시작 실패: {error}"))
        })?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            NoaError::AndroidUnavailable(format!("ART 사전 검사 상태 확인 실패: {error}"))
        })? {
            return if status.success() {
                Ok(())
            } else {
                Err(NoaError::AndroidUnavailable(format!(
                    "ART 사전 검사 프로세스가 비정상 종료되었습니다: {status}"
                )))
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(NoaError::AndroidUnavailable(
                "ART 사전 검사가 15초 안에 완료되지 않았습니다".to_string(),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(not(target_os = "android"))]
fn unavailable<T>() -> Result<T, NoaError> {
    Err(NoaError::AndroidUnavailable(
        "호스트 빌드에서는 KakaoTalk 전송을 실행할 수 없습니다".to_string(),
    ))
}
