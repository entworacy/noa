use serde::Deserialize;

use crate::failure::NoaError;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum VoxSessionKind {
    Cecall,
    Voiceroom,
}

impl VoxSessionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Cecall => "cecall",
            Self::Voiceroom => "voiceroom",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct ExpectedSession {
    pub(super) kind: Option<VoxSessionKind>,
    pub(super) chat_id: Option<i64>,
}

pub(super) fn ensure_active_session(
    status: &serde_json::Value,
    expected: ExpectedSession,
) -> Result<(), NoaError> {
    if status
        .get("moduleLoaded")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        let detail = status
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("VOX 모듈 상태를 확인할 수 없습니다");
        return Err(NoaError::AndroidUnavailable(detail.to_string()));
    }

    let matches = match expected.kind {
        Some(kind) => session_matches(status.get(kind.as_str()), expected.chat_id),
        None => {
            session_matches(status.get("cecall"), expected.chat_id)
                || session_matches(status.get("voiceroom"), expected.chat_id)
        }
    };
    if matches {
        return Ok(());
    }

    let kind = expected.kind.map(VoxSessionKind::as_str).unwrap_or("VOX");
    let target = expected
        .chat_id
        .map(|chat_id| format!(" (chatId={chat_id})"))
        .unwrap_or_default();
    Err(NoaError::Conflict(format!(
        "{kind} 세션이 서버에서 종료되었거나 다른 세션으로 변경되었습니다{target}"
    )))
}

pub(super) fn parse_positive_chat_id(value: &str) -> Result<i64, NoaError> {
    value
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| NoaError::BadRequest("chatId는 0보다 큰 정수여야 합니다".to_string()))
}

fn session_matches(session: Option<&serde_json::Value>, expected_chat_id: Option<i64>) -> bool {
    let Some(session) = session else {
        return false;
    };
    if session.get("idle").and_then(serde_json::Value::as_bool) != Some(false) {
        return false;
    }
    expected_chat_id.is_none_or(|expected| {
        session
            .get("chatId")
            .and_then(json_i64)
            .is_some_and(|actual| actual == expected)
    })
}

fn json_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.parse::<i64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_session_must_match_kind_and_chat_id() {
        let status = serde_json::json!({
            "moduleLoaded": true,
            "cecall": {"idle": true, "chatId": null},
            "voiceroom": {"idle": false, "chatId": "10"},
            "audio": {"queuedBytes": 0}
        });
        let expected = ExpectedSession {
            kind: Some(VoxSessionKind::Voiceroom),
            chat_id: Some(10),
        };
        assert!(ensure_active_session(&status, expected).is_ok());

        let wrong_room = ExpectedSession {
            kind: Some(VoxSessionKind::Voiceroom),
            chat_id: Some(11),
        };
        assert!(matches!(
            ensure_active_session(&status, wrong_room),
            Err(NoaError::Conflict(_))
        ));
    }

    #[test]
    fn idle_or_unloaded_vox_is_rejected() {
        let idle = serde_json::json!({
            "moduleLoaded": true,
            "cecall": {"idle": true},
            "voiceroom": {"idle": true}
        });
        assert!(matches!(
            ensure_active_session(
                &idle,
                ExpectedSession {
                    kind: None,
                    chat_id: None,
                }
            ),
            Err(NoaError::Conflict(_))
        ));

        let unloaded = serde_json::json!({
            "moduleLoaded": false,
            "error": "not loaded"
        });
        assert!(matches!(
            ensure_active_session(
                &unloaded,
                ExpectedSession {
                    kind: None,
                    chat_id: None,
                }
            ),
            Err(NoaError::AndroidUnavailable(_))
        ));
    }
}
