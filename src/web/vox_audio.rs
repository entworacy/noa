use actix_web::{HttpRequest, HttpResponse, Responder, web};
use futures_util::StreamExt;
use serde::Deserialize;
use std::time::Duration;
use tokio::time::Instant;

use super::{AppState, authorize, vox::ensure_vox_enabled};
use crate::failure::NoaError;
use session::{ExpectedSession, VoxSessionKind, ensure_active_session, parse_positive_chat_id};

mod session;

const MAX_PCM_CHUNK_BYTES: usize = 96_000;
const STREAM_PCM_CHUNK_BYTES: usize = 9_600;
const PCM_BYTES_PER_SECOND: u64 = 96_000;
const STREAM_TARGET_QUEUE_BYTES: u64 = 48_000;
const STREAM_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const STREAM_QUEUE_POLL_INTERVAL: Duration = Duration::from_millis(20);
const STREAM_SESSION_POLL_INTERVAL: Duration = Duration::from_millis(250);

pub(super) fn configure(config: &mut web::ServiceConfig) {
    config
        .route("/api/vox/audio/start", web::post().to(start))
        .route("/api/vox/audio", web::post().to(push))
        .route("/api/vox/audio/stream", web::post().to(stream))
        .route("/api/vox/audio/stop", web::post().to(stop));
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AudioMode {
    Replace,
    Mix,
}

impl AudioMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Mix => "mix",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioStartRequest {
    mode: Option<AudioMode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AudioStreamQuery {
    mode: Option<AudioMode>,
    kind: Option<VoxSessionKind>,
    chat_id: Option<String>,
}

async fn start(
    req: HttpRequest,
    body: web::Json<AudioStartRequest>,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    ensure_vox_enabled(&state)?;
    let mode = body.mode.unwrap_or(AudioMode::Replace);
    Ok(web::Json(
        crate::intercept::vox_audio_start(mode.as_str().to_string()).await?,
    ))
}

async fn push(
    req: HttpRequest,
    body: web::Bytes,
    state: web::Data<AppState>,
) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    ensure_vox_enabled(&state)?;
    validate_pcm_chunk(&body)?;
    Ok(web::Json(
        crate::intercept::vox_audio_push(body.to_vec()).await?,
    ))
}

async fn stream(
    req: HttpRequest,
    query: web::Query<AudioStreamQuery>,
    mut payload: web::Payload,
    state: web::Data<AppState>,
) -> Result<HttpResponse, NoaError> {
    authorize(&req, &state)?;
    ensure_vox_enabled(&state)?;
    let mode = query.mode.unwrap_or(AudioMode::Replace);
    let expected = ExpectedSession {
        kind: query.kind,
        chat_id: query
            .chat_id
            .as_deref()
            .map(parse_positive_chat_id)
            .transpose()?,
    };
    ensure_active_session(&crate::intercept::vox_status().await?, expected)?;

    let streamed = match stream_pcm(&mut payload, mode, expected).await {
        Ok(bytes) => drain_audio_queue(expected).await.map(|()| bytes),
        Err(error) => Err(error),
    };
    let stopped = crate::intercept::vox_audio_stop().await;
    match streamed {
        Ok(bytes) => Ok(HttpResponse::Ok().json(serde_json::json!({
            "ok": true,
            "mode": mode.as_str(),
            "paced": true,
            "drained": true,
            "streamedBytes": bytes,
            "audio": stopped?
        }))),
        Err(error) => {
            if let Err(stop_error) = stopped {
                tracing::warn!(%stop_error, "VOX PCM 스트림 오류 후 송출 중지 실패");
            }
            Err(error)
        }
    }
}

async fn stream_pcm(
    payload: &mut web::Payload,
    mode: AudioMode,
    expected: ExpectedSession,
) -> Result<u64, NoaError> {
    let mut streamed = 0_u64;
    let mut carry = None;
    let mut started = false;
    let mut pacer = RealtimePacer::new();
    let mut session_poll = tokio::time::interval_at(
        Instant::now() + STREAM_SESSION_POLL_INTERVAL,
        STREAM_SESSION_POLL_INTERVAL,
    );
    session_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let item = tokio::select! {
            item = payload.next() => item,
            _ = session_poll.tick() => {
                ensure_active_session(&crate::intercept::vox_status().await?, expected)?;
                continue;
            }
        };
        let Some(item) = item else {
            break;
        };
        let bytes =
            item.map_err(|error| NoaError::BadRequest(format!("PCM 스트림 읽기 실패: {error}")))?;
        if bytes.is_empty() {
            continue;
        }
        let mut offset = 0;
        if let Some(low) = carry.take() {
            push_stream_chunk(
                &mut pacer,
                &mut started,
                mode,
                expected,
                vec![low, bytes[0]],
            )
            .await?;
            streamed = streamed.saturating_add(2);
            offset = 1;
        }
        let remaining = &bytes[offset..];
        let even_length = remaining.len() & !1;
        for chunk in remaining[..even_length].chunks(STREAM_PCM_CHUNK_BYTES) {
            push_stream_chunk(&mut pacer, &mut started, mode, expected, chunk.to_vec()).await?;
            streamed = streamed.saturating_add(chunk.len() as u64);
        }
        if even_length != remaining.len() {
            carry = remaining.last().copied();
        }
    }
    if carry.is_some() {
        return Err(NoaError::BadRequest(
            "PCM 스트림이 불완전한 16-bit 샘플로 끝났습니다".to_string(),
        ));
    }
    if streamed == 0 {
        return Err(NoaError::BadRequest(
            "PCM 스트림이 비어 있습니다".to_string(),
        ));
    }
    Ok(streamed)
}

struct RealtimePacer {
    started: Instant,
    pushed_bytes: u64,
}

impl RealtimePacer {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            pushed_bytes: 0,
        }
    }

    async fn pushed(&mut self, bytes: usize) {
        self.pushed_bytes = self.pushed_bytes.saturating_add(bytes as u64);
        tokio::time::sleep_until(self.started + playback_offset(self.pushed_bytes)).await;
    }
}

async fn push_stream_chunk(
    pacer: &mut RealtimePacer,
    started: &mut bool,
    mode: AudioMode,
    expected: ExpectedSession,
    bytes: Vec<u8>,
) -> Result<(), NoaError> {
    if !*started {
        crate::intercept::vox_audio_start(mode.as_str().to_string()).await?;
        *started = true;
    }
    let length = bytes.len();
    let status = crate::intercept::vox_audio_push(bytes).await?;
    pacer.pushed(length).await;
    if audio_queue_bytes(&status)? > STREAM_TARGET_QUEUE_BYTES {
        wait_for_audio_queue(STREAM_TARGET_QUEUE_BYTES, expected).await?;
    }
    Ok(())
}

async fn drain_audio_queue(expected: ExpectedSession) -> Result<(), NoaError> {
    wait_for_audio_queue(0, expected).await
}

async fn wait_for_audio_queue(max_bytes: u64, expected: ExpectedSession) -> Result<(), NoaError> {
    let deadline = Instant::now() + STREAM_DRAIN_TIMEOUT;
    loop {
        let status = crate::intercept::vox_status().await?;
        ensure_active_session(&status, expected)?;
        if audio_queue_bytes(&status)? <= max_bytes {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(NoaError::Internal(format!(
                "VOX PCM queue가 {STREAM_DRAIN_TIMEOUT:?} 안에 {max_bytes}바이트 이하로 비워지지 않았습니다"
            )));
        }
        tokio::time::sleep(STREAM_QUEUE_POLL_INTERVAL).await;
    }
}

fn audio_queue_bytes(status: &serde_json::Value) -> Result<u64, NoaError> {
    status
        .get("queuedBytes")
        .or_else(|| status.pointer("/audio/queuedBytes"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            NoaError::Internal("VOX PCM queue 상태에 queuedBytes가 없습니다".to_string())
        })
}

fn playback_offset(bytes: u64) -> Duration {
    let nanos = (u128::from(bytes) * 1_000_000_000_u128) / u128::from(PCM_BYTES_PER_SECOND);
    Duration::from_nanos(nanos.min(u128::from(u64::MAX)) as u64)
}

async fn stop(req: HttpRequest, state: web::Data<AppState>) -> Result<impl Responder, NoaError> {
    authorize(&req, &state)?;
    ensure_vox_enabled(&state)?;
    Ok(web::Json(crate::intercept::vox_audio_stop().await?))
}

fn validate_pcm_chunk(bytes: &[u8]) -> Result<(), NoaError> {
    if bytes.is_empty() {
        return Err(NoaError::BadRequest("PCM 청크가 비어 있습니다".to_string()));
    }
    if bytes.len() > MAX_PCM_CHUNK_BYTES {
        return Err(NoaError::BadRequest(format!(
            "PCM 청크는 {MAX_PCM_CHUNK_BYTES}바이트 이하여야 합니다"
        )));
    }
    if !bytes.len().is_multiple_of(2) {
        return Err(NoaError::BadRequest(
            "PCM 청크는 완전한 16-bit 샘플이어야 합니다".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_chunks_require_complete_bounded_samples() {
        assert!(validate_pcm_chunk(&[0, 0]).is_ok());
        assert!(validate_pcm_chunk(&[0]).is_err());
        assert!(validate_pcm_chunk(&vec![0; MAX_PCM_CHUNK_BYTES + 2]).is_err());
    }

    #[test]
    fn stream_chunks_are_paced_at_the_pcm_byte_rate() {
        assert_eq!(STREAM_PCM_CHUNK_BYTES, 9_600);
        assert_eq!(playback_offset(9_600), Duration::from_millis(100));
        assert_eq!(playback_offset(96_000), Duration::from_secs(1));
    }

    #[test]
    fn queue_depth_is_read_from_push_and_status_responses() {
        assert_eq!(
            audio_queue_bytes(&serde_json::json!({"queuedBytes": 9_600})).unwrap(),
            9_600
        );
        assert_eq!(
            audio_queue_bytes(&serde_json::json!({"audio": {"queuedBytes": 4_800}})).unwrap(),
            4_800
        );
    }
}
