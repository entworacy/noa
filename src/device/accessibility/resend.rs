use std::time::{Duration, Instant};

use crate::failure::NoaError;

#[cfg(target_os = "android")]
use super::CHAT_TITLE_ID;
use super::{
    Bounds, CHAT_ACTION, CHAT_ACTIVITY, UiNode, matches_label, run, select_chat_title, tap,
    tap_labeled, wait_for,
};

const RESEND_LABELS: [&str; 2] = ["재전송", "Re-send"];
const RESEND_INDICATOR_ID: &str = "com.kakao.talk:id/resend_indicator";
const DIRECT_RESEND_ID: &str = "com.kakao.talk:id/circle_progress_layout";
const BUBBLE_LINE_ID: &str = "com.kakao.talk:id/bubble_linearlayout";
const MESSAGE_BUBBLE_ID: &str = "com.kakao.talk:id/bubble";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResendClickMode {
    Confirmation,
    Direct,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResendTarget {
    bounds: Bounds,
    mode: ResendClickMode,
}

pub fn resend(
    room_id: i64,
    profile: i32,
    expected_room_name: &str,
    message: &str,
    attachment: &str,
) -> Result<(), NoaError> {
    let started = Instant::now();
    let profile = profile.to_string();
    let room = room_id.to_string();
    run(
        "/system/bin/am",
        &[
            "start",
            "--user",
            &profile,
            "-S",
            "-n",
            CHAT_ACTIVITY,
            "-a",
            CHAT_ACTION,
            "-f",
            "335544320",
            "--el",
            "chatRoomId",
            &room,
        ],
    )?;
    let activity_ms = started.elapsed().as_millis();
    let stage = Instant::now();
    verify_room(expected_room_name)?;
    let room_verification_ms = stage.elapsed().as_millis();
    let stage = Instant::now();
    let resend_mode = click_matching_target(message, attachment)?;
    let target_click_ms = stage.elapsed().as_millis();
    let stage = Instant::now();

    let confirmation_mode = if resend_mode == ResendClickMode::Direct {
        "direct"
    } else {
        #[cfg(target_os = "android")]
        {
            match super::super::ui_agent::click_resend_confirmation() {
                Ok(true) => "agent",
                Ok(false) => {
                    return Err(NoaError::AndroidUnavailable(
                        "재전송 확인 버튼을 찾지 못했습니다".to_string(),
                    ));
                }
                Err(error) => {
                    tracing::warn!(%error, "재전송 확인 버튼 빠른 선택 실패, 화면 덤프로 전환합니다");
                    "xml"
                }
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            "xml"
        }
    };

    if confirmation_mode == "xml" {
        let confirmation = wait_for(Duration::from_secs(8), |nodes| {
            nodes
                .iter()
                .filter(|node| matches_label(node, &RESEND_LABELS))
                .filter_map(|node| node.bounds)
                .next()
        })
        .ok_or_else(|| {
            NoaError::AndroidUnavailable("재전송 확인 버튼을 찾지 못했습니다".to_string())
        })?;
        tap_labeled(confirmation, &RESEND_LABELS)?;
    }

    tracing::info!(
        room_id,
        activity_ms,
        room_verification_ms,
        target_click_ms,
        confirmation_ms = stage.elapsed().as_millis(),
        total_ms = started.elapsed().as_millis(),
        confirmation_mode,
        "custom 접근성 재전송 단계별 소요시간"
    );
    Ok(())
}

fn verify_room(expected_room_name: &str) -> Result<(), NoaError> {
    #[cfg(target_os = "android")]
    match super::super::ui_agent::wait_for_resource_text(
        CHAT_TITLE_ID,
        expected_room_name,
        Duration::from_secs(8),
    ) {
        Ok(true) => return Ok(()),
        Ok(false) => {
            return Err(NoaError::AndroidUnavailable(format!(
                "재전송할 채팅방 화면을 확인하지 못했습니다: {expected_room_name}"
            )));
        }
        Err(error) => {
            tracing::warn!(%error, "재전송 채팅방 빠른 검증 실패, 화면 덤프로 전환합니다");
        }
    }

    wait_for(Duration::from_secs(8), |nodes| {
        select_chat_title(nodes)
            .is_some_and(|title| title.trim() == expected_room_name.trim())
            .then_some(())
    })
    .ok_or_else(|| {
        NoaError::AndroidUnavailable(format!(
            "재전송할 채팅방 화면을 확인하지 못했습니다: {expected_room_name}"
        ))
    })
}

fn click_matching_target(message: &str, attachment: &str) -> Result<ResendClickMode, NoaError> {
    #[cfg(target_os = "android")]
    let mut fallback_timeout = Duration::from_secs(15);
    #[cfg(not(target_os = "android"))]
    let fallback_timeout = Duration::from_secs(15);
    #[cfg(target_os = "android")]
    match super::super::ui_agent::click_resend_target(
        &target_strings(message, attachment),
        Duration::from_secs(15),
    ) {
        Ok(super::super::ui_agent::ResendTargetClickResult::ClickedConfirmation) => {
            return Ok(ResendClickMode::Confirmation);
        }
        Ok(super::super::ui_agent::ResendTargetClickResult::ClickedDirect) => {
            return Ok(ResendClickMode::Direct);
        }
        Ok(super::super::ui_agent::ResendTargetClickResult::NotFound) => {
            tracing::warn!(
                "대상 메시지 재전송 대상을 빠른 탐색에서 찾지 못해 화면 덤프로 검증합니다"
            );
            fallback_timeout = Duration::from_secs(3);
        }
        Ok(super::super::ui_agent::ResendTargetClickResult::Ambiguous) => {
            tracing::warn!("재전송 대상이 여러 개라 화면 덤프로 대상 버블을 검증합니다");
            fallback_timeout = Duration::from_secs(3);
        }
        Err(error) => {
            tracing::warn!(%error, "재전송 대상 빠른 선택 실패, 화면 덤프로 전환합니다");
        }
    }

    let target = wait_for(fallback_timeout, |nodes| {
        select_target(nodes, message, attachment)
    })
    .ok_or_else(|| {
        NoaError::AndroidUnavailable(
            "대상 메시지와 일치하는 재전송 대상을 찾지 못했습니다".to_string(),
        )
    })?;
    tap_target(target)
}

fn tap_target(target: ResendTarget) -> Result<ResendClickMode, NoaError> {
    #[cfg(target_os = "android")]
    let resource_id = match target.mode {
        ResendClickMode::Confirmation => RESEND_INDICATOR_ID,
        ResendClickMode::Direct => DIRECT_RESEND_ID,
    };
    #[cfg(target_os = "android")]
    match super::super::ui_agent::click_resource_at(resource_id, target.bounds) {
        Ok(true) => return Ok(target.mode),
        Ok(false) => {
            return Err(NoaError::AndroidUnavailable(
                "클릭 직전에 재전송 대상이 사라졌습니다".to_string(),
            ));
        }
        Err(error) => {
            tracing::warn!(%error, "재전송 대상 의미 기반 클릭 실패, 좌표 폴백을 사용합니다");
        }
    }
    tap(target.bounds)?;
    Ok(target.mode)
}

fn select_target(nodes: &[UiNode], message: &str, attachment: &str) -> Option<ResendTarget> {
    let targets = target_strings(message, attachment);
    let candidates: Vec<(usize, ResendTarget)> = nodes
        .iter()
        .filter_map(|node| {
            let (mode, container_id) = match node.resource_id.as_str() {
                RESEND_INDICATOR_ID => (ResendClickMode::Confirmation, BUBBLE_LINE_ID),
                DIRECT_RESEND_ID => (ResendClickMode::Direct, MESSAGE_BUBBLE_ID),
                _ => return None,
            };
            let bounds = node.bounds?;
            let bubble_bounds = nodes
                .iter()
                .filter(|candidate| candidate.resource_id == container_id)
                .filter_map(|candidate| candidate.bounds)
                .filter(|bubble| contains(*bubble, bounds))
                .min_by_key(|bubble| area(*bubble))?;
            let score = nodes
                .iter()
                .filter(|node| {
                    node.bounds
                        .is_some_and(|value| contains(bubble_bounds, value))
                })
                .flat_map(|node| [&node.text, &node.description])
                .filter(|value| !value.is_empty())
                .map(|value| match_score(value, &targets))
                .sum::<usize>();
            Some((score, ResendTarget { bounds, mode }))
        })
        .collect();
    if targets.is_empty() {
        return match candidates.as_slice() {
            [(_, target)] => Some(*target),
            _ => None,
        };
    }
    candidates
        .into_iter()
        .filter(|(score, _)| *score > 0)
        .max_by_key(|(score, target)| {
            (
                *score,
                target.bounds.3,
                usize::from(target.mode == ResendClickMode::Direct),
            )
        })
        .map(|(_, target)| target)
}

fn target_strings(message: &str, attachment: &str) -> Vec<String> {
    let mut values = Vec::new();
    push_target(&mut values, message);
    for part in message.split(|character: char| {
        character.is_whitespace() || matches!(character, ':' | '#' | ',' | '(' | ')')
    }) {
        push_target(&mut values, part);
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(attachment) {
        collect_json_strings(&value, &mut values);
    }
    values.sort();
    values.dedup();
    values
}

fn collect_json_strings(value: &serde_json::Value, output: &mut Vec<String>) {
    match value {
        serde_json::Value::String(value) => push_target(output, value),
        serde_json::Value::Array(values) => {
            for value in values {
                collect_json_strings(value, output);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                collect_json_strings(value, output);
            }
        }
        _ => {}
    }
}

fn push_target(output: &mut Vec<String>, value: &str) {
    let value = value.trim();
    let length = value.chars().count();
    if (2..=100).contains(&length) && !value.starts_with("http") {
        output.push(value.to_lowercase());
    }
}

fn match_score(value: &str, targets: &[String]) -> usize {
    let value = value.trim().to_lowercase();
    targets
        .iter()
        .map(|target| {
            if value == *target {
                1_000 + target.chars().count()
            } else if value.contains(target) || target.contains(&value) {
                target.chars().count().min(value.chars().count())
            } else {
                0
            }
        })
        .max()
        .unwrap_or_default()
}

fn contains(outer: Bounds, inner: Bounds) -> bool {
    outer.0 <= inner.0 && outer.1 <= inner.1 && outer.2 >= inner.2 && outer.3 >= inner.3
}

fn area(bounds: Bounds) -> i64 {
    i64::from(bounds.2 - bounds.0) * i64::from(bounds.3 - bounds.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::accessibility::parse_nodes;

    #[test]
    fn labels_cover_korean_and_english() {
        assert!(!RESEND_LABELS[0].is_ascii());
        assert!(RESEND_LABELS[1].is_ascii());
    }

    #[test]
    fn selects_confirmation_indicator_from_matching_bubble() {
        let xml = r#"<hierarchy><node resource-id="com.kakao.talk:id/bubble_linearlayout" text="" content-desc="" bounds="[0,10][720,300]"><node resource-id="com.kakao.talk:id/resend_indicator" text="" content-desc="Delete or resend failed message" bounds="[100,250][200,290]"/><node resource-id="" text="서울 날씨" content-desc="" bounds="[220,20][700,240]"/></node><node resource-id="com.kakao.talk:id/bubble_linearlayout" text="" content-desc="" bounds="[0,310][720,1000]"><node resource-id="com.kakao.talk:id/resend_indicator" text="" content-desc="Delete or resend failed message" bounds="[100,940][200,990]"/><node resource-id="" text="샵검색" content-desc="" bounds="[220,330][700,900]"/></node></hierarchy>"#;
        let nodes = parse_nodes(xml);
        assert_eq!(
            select_target(&nodes, "서울특별시 내일 날씨", r#"{"title":"서울 날씨"}"#),
            Some(ResendTarget {
                bounds: (100, 250, 200, 290),
                mode: ResendClickMode::Confirmation,
            })
        );
        assert_eq!(
            select_target(&nodes, "샵검색: #샵검색", r#"{"title":"샵검색"}"#),
            Some(ResendTarget {
                bounds: (100, 940, 200, 990),
                mode: ResendClickMode::Confirmation,
            })
        );
        assert_eq!(
            select_target(&nodes, "일치하지 않는 메시지", r#"{"title":"다른 값"}"#),
            None
        );
        assert_eq!(select_target(&nodes, "", "{}"), None);

        let single = parse_nodes(
            r#"<hierarchy><node resource-id="com.kakao.talk:id/bubble_linearlayout" text="" content-desc="" bounds="[0,10][720,300]"><node resource-id="com.kakao.talk:id/resend_indicator" text="" content-desc="Delete or resend failed message" bounds="[100,250][200,290]"/></node></hierarchy>"#,
        );
        assert_eq!(
            select_target(&single, "", "{}"),
            Some(ResendTarget {
                bounds: (100, 250, 200, 290),
                mode: ResendClickMode::Confirmation,
            })
        );
    }

    #[test]
    fn selects_direct_retry_control_from_message_bubble() {
        let xml = r#"<hierarchy><node resource-id="com.kakao.talk:id/bubble" text="" content-desc="@붸엙 님 안녕하세요!" bounds="[220,10][700,300]"><node resource-id="com.kakao.talk:id/circle_progress_layout" text="" content-desc="Re-send" bounds="[600,220][680,290]"/></node><node resource-id="com.kakao.talk:id/bubble" text="" content-desc="다른 실패 메시지" bounds="[220,310][700,600]"><node resource-id="com.kakao.talk:id/circle_progress_layout" text="" content-desc="Re-send" bounds="[600,520][680,590]"/></node></hierarchy>"#;
        let nodes = parse_nodes(xml);
        assert_eq!(
            select_target(&nodes, "@붸엙 님 안녕하세요!", r#"{"mentions":[1]}"#),
            Some(ResendTarget {
                bounds: (600, 220, 680, 290),
                mode: ResendClickMode::Direct,
            })
        );
        assert_eq!(
            select_target(&nodes, "전혀일치하지않는고유문자열", "{}"),
            None
        );
    }
}
