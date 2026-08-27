use std::{
    fs,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use crate::failure::NoaError;

// dump 파일 1개를 계속 덮어써서 화면당 임시 파일 증가는 0개로 확인 test24
const UI_DUMP: &str = "/data/local/tmp/noa-accessibility.xml";
// 1000 / 16 = 초당 최대 62회 확인이고 실제 37ms dump까지 합치면 초당 약 18회 예상 test25
const STATE_POLL_INTERVAL: Duration = Duration::from_millis(16);
// APK 26 6 3의 ko en split에서 확인한 11쌍이라 총 문자열은 22개로 계산 test26
type LabelPair = [&'static str; 2];
const JOIN_OPEN_CHAT_LABELS: LabelPair = ["오픈채팅 참여하기", "Join Open Chat"];
const SETTINGS_LABELS: LabelPair = ["설정", "Settings"];
const LEAVE_CHATROOM_LABELS: LabelPair = ["채팅방 나가기", "Leave chatroom"];
const LEAVE_LABELS: LabelPair = ["나가기", "Leave"];
const KICK_MEMBER_LABELS: LabelPair = ["대화상대 내보내기", "Send out participant"];
const REMOVE_LABELS: LabelPair = ["내보내기", "Remove"];
const RESEND_LABELS: LabelPair = ["재전송", "Re-send"];
#[cfg(not(target_os = "android"))]
const EXPAND_MEMBER_LABELS: LabelPair = ["펼치기", "Expand"];
#[cfg(not(target_os = "android"))]
const COPY_LABELS: LabelPair = ["링크 복사", "Copy Link"];
const PARTICIPANTS_LABELS: LabelPair = ["대화상대", "Participants"];
const SELF_LABELS: LabelPair = ["나", "me"];
const KAKAO_FRIENDS_LABELS: LabelPair = ["카카오프렌즈", "Kakao Friends"];
const NEW_OPEN_PROFILE_LABELS: LabelPair = ["새 오픈프로필", "New Open Profile"];

const KAKAO_PACKAGE: &str = "com.kakao.talk";
const CHAT_ACTIVITY: &str = "com.kakao.talk/.activity.RecentExcludeIntentFilterActivity";
const CHAT_ACTION: &str = "com.kakao.talk.intent.action.ENTER_CHAT_ROOM";
const CHAT_TITLE_ID: &str = "com.kakao.talk:id/toolbar_default_title_text";
const RESEND_INDICATOR_ID: &str = "com.kakao.talk:id/resend_indicator";
const OPEN_PROFILE_ACTIVITY: &str =
    "com.kakao.talk/com.kakao.talk.openlink.openprofile.viewer.OlkOpenProfileViewerActivity";
const OPEN_PROFILE_NAME_ID: &str = "com.kakao.talk.openlink:id/name";
#[cfg(not(target_os = "android"))]
const OPEN_PROFILE_MORE_ID: &str = "com.kakao.talk.openlink:id/toolbar_more";
const OPEN_CHAT_JOIN_ID: &str = "com.kakao.talk.openlink:id/join_layout";
const SETTING_BUTTON_ID: &str = "com.kakao.talk:id/setting_button";
const OPEN_CHAT_MESSAGE_ID: &str = "com.kakao.talk:id/txt_message";
const OPEN_CHAT_COVER_TITLE_ID: &str = "com.kakao.talk.openlink:id/title";
const OPEN_CHAT_COVER_TITLE_PREFIX: &str = "com.kakao.talk.openlink:id/title_res_";
const OPEN_CHAT_PROFILE_NAME_ID: &str = "com.kakao.talk.openlink:id/profile_name";
const OPEN_CHAT_PROFILE_NAME_PREFIX: &str = "com.kakao.talk.openlink:id/profile_name_res_";
const MEMBER_INFO_ACTIVITY: &str =
    "com.kakao.talk/.activity.chatroom.chatside.ChatRoomSideActivity";
// left top right bottom 순서의 좌표 4개로 클릭 영역 1개 표현 test27
type Bounds = (i32, i32, i32, i32);

enum JoinDestination {
    Entered(String),
    Profile(String, Option<Bounds>),
}

enum OpenChatDestination {
    Entered(String),
    Cover(String, Option<Bounds>),
}

pub fn join_open_chat(
    url: &str,
    profile: i32,
    selected_profile: &str,
) -> Result<(String, Option<String>), NoaError> {
    // cover 20초 + 선택 12초 + 입장 확인 20초라 최악 경로 상한은 52초 test28
    let scheme = open_chat_scheme(url)?;
    let user = profile.to_string();
    run(
        "/system/bin/am",
        &[
            "start",
            "--user",
            &user,
            "-f",
            "335544320",
            "-a",
            "android.intent.action.VIEW",
            "-d",
            &scheme,
            KAKAO_PACKAGE,
        ],
    )?;
    let destination = wait_for_open_chat_destination(Duration::from_secs(20))?;
    let OpenChatDestination::Cover(room_name, join_button) = destination else {
        let OpenChatDestination::Entered(room_name) = destination else {
            unreachable!();
        };
        return Ok((room_name, None));
    };
    if let Some(join_button) = join_button {
        tap(join_button)?;
    } else {
        #[cfg(target_os = "android")]
        if !super::ui_agent::click_resource(OPEN_CHAT_JOIN_ID)
            .map_err(NoaError::AndroidUnavailable)?
        {
            return Err(NoaError::AndroidUnavailable(
                "오픈채팅 참여 버튼이 클릭 전에 사라졌습니다".to_string(),
            ));
        }
    }
    let destination = wait_for_open_chat_profile(selected_profile, Duration::from_secs(12))?;
    let (selected_profile, profile_button) = match destination {
        JoinDestination::Entered(entered) if entered == room_name => {
            return Ok((room_name, None));
        }
        JoinDestination::Entered(entered) => {
            return Err(NoaError::AndroidUnavailable(format!(
                "다른 채팅방이 열렸습니다: {entered}"
            )));
        }
        JoinDestination::Profile(name, bounds) => (name, bounds),
    };
    if let Some(profile_button) = profile_button {
        tap_named(profile_button, &selected_profile)?;
    }
    let entered_room = wait_for_open_chat_entered(Duration::from_secs(20))?;
    if entered_room != room_name {
        return Err(NoaError::AndroidUnavailable(format!(
            "다른 채팅방이 열렸습니다: {entered_room}"
        )));
    }
    Ok((room_name, Some(selected_profile)))
}

pub fn share_open_profile(
    link_id: i64,
    profile: i32,
    expected_url: Option<&str>,
) -> Result<String, NoaError> {
    if expected_url.is_some_and(|url| !is_open_profile_url(url)) {
        return Err(NoaError::BadRequest(
            "오픈프로필 공유 링크가 올바르지 않습니다".to_string(),
        ));
    }
    open_profile_activity(link_id, profile)?;

    #[cfg(target_os = "android")]
    {
        super::ui_agent::prepare_clipboard().map_err(NoaError::AndroidUnavailable)?;
        if !super::ui_agent::click_open_profile_more().map_err(NoaError::AndroidUnavailable)? {
            tracing::warn!(
                link_id,
                "오픈프로필 화면을 다시 열어 공유 메뉴 클릭을 재시도합니다"
            );
            open_profile_activity(link_id, profile)?;
            if !super::ui_agent::click_open_profile_more().map_err(NoaError::AndroidUnavailable)? {
                return Err(NoaError::AndroidUnavailable(
                    "오픈프로필 공유 메뉴가 클릭 전에 사라졌습니다".to_string(),
                ));
            }
        }
        if !super::ui_agent::click_open_link_copy().map_err(NoaError::AndroidUnavailable)? {
            return Err(NoaError::AndroidUnavailable(
                "오픈프로필 링크 복사 항목을 찾지 못했습니다".to_string(),
            ));
        }
        let copied = super::ui_agent::wait_clipboard_change(Duration::from_secs(5))
            .map_err(NoaError::AndroidUnavailable)?
            .ok_or_else(|| {
                NoaError::AndroidUnavailable(
                    "오픈프로필 링크 복사 후 클립보드가 변경되지 않았습니다".to_string(),
                )
            })?;
        return verify_open_profile_url(&copied, expected_url);
    }

    #[cfg(not(target_os = "android"))]
    {
        let more = wait_for(Duration::from_secs(12), |nodes| {
            nodes
                .iter()
                .find(|node| node.resource_id == OPEN_PROFILE_MORE_ID)
                .and_then(|node| node.bounds)
        })
        .ok_or_else(|| {
            NoaError::AndroidUnavailable("오픈프로필 공유 메뉴를 찾지 못했습니다".to_string())
        })?;
        tap(more)?;
        let copy = wait_for(Duration::from_secs(5), |nodes| {
            nodes
                .iter()
                .find(|node| matches_label(node, &COPY_LABELS))
                .and_then(|node| node.bounds)
        })
        .ok_or_else(|| {
            NoaError::AndroidUnavailable("오픈프로필 링크 복사 항목을 찾지 못했습니다".to_string())
        })?;
        tap(copy)?;
    }

    #[cfg(not(target_os = "android"))]
    expected_url.map(str::to_string).ok_or_else(|| {
        NoaError::AndroidUnavailable(
            "호스트 환경에서는 복사된 오픈프로필 링크를 읽을 수 없습니다".to_string(),
        )
    })
}

pub fn share_member_open_profile(
    room_id: i64,
    profile: i32,
    room_name: &str,
    nickname: &str,
) -> Result<String, NoaError> {
    #[cfg(target_os = "android")]
    super::ui_agent::prepare_clipboard().map_err(NoaError::AndroidUnavailable)?;

    open_member_info(room_id, profile, room_name)?;
    select_member_profile(nickname)?;

    #[cfg(target_os = "android")]
    {
        if !wait_for_member_profile_focus(Duration::from_secs(8)) {
            return Err(NoaError::AndroidUnavailable(
                "참여자 선택 후 프로필 Activity가 열리지 않았습니다".to_string(),
            ));
        }
        if !super::ui_agent::wait_for_resource_text(
            OPEN_PROFILE_NAME_ID,
            nickname,
            Duration::from_secs(8),
        )
        .map_err(NoaError::AndroidUnavailable)?
        {
            return Err(NoaError::AndroidUnavailable(
                "열린 프로필이 요청한 참여자와 일치하지 않습니다".to_string(),
            ));
        }
        match super::ui_agent::wait_member_profile_share().map_err(NoaError::AndroidUnavailable)? {
            super::ui_agent::MemberProfileShareState::Shareable => {}
            super::ui_agent::MemberProfileShareState::NotShareable => {
                return Err(NoaError::NotFound(
                    "선택한 참여자는 오픈채팅 프로필을 사용하지만 링크 공유가 가능한 독립 오픈프로필은 아닙니다"
                        .to_string(),
                ));
            }
            super::ui_agent::MemberProfileShareState::Unknown => {
                return Err(NoaError::NotFound(
                    "선택한 참여자의 프로필에는 링크 공유 메뉴가 없어 공유 가능한 오픈프로필 URL을 확인할 수 없습니다"
                        .to_string(),
                ));
            }
        }
        if !super::ui_agent::click_open_profile_more().map_err(NoaError::AndroidUnavailable)? {
            return Err(NoaError::AndroidUnavailable(
                "오픈프로필 공유 메뉴가 클릭 전에 사라졌습니다".to_string(),
            ));
        }
        if !super::ui_agent::click_open_link_copy().map_err(NoaError::AndroidUnavailable)? {
            return Err(NoaError::NotFound(
                "선택한 참여자의 프로필에서 링크 복사 항목을 찾지 못했습니다".to_string(),
            ));
        }
        let copied = super::ui_agent::wait_clipboard_change(Duration::from_secs(5))
            .map_err(NoaError::AndroidUnavailable)?
            .ok_or_else(|| {
                NoaError::AndroidUnavailable(
                    "오픈프로필 링크 복사 후 클립보드가 변경되지 않았습니다".to_string(),
                )
            })?;
        return verify_open_profile_url(&copied, None);
    }

    #[cfg(not(target_os = "android"))]
    Err(NoaError::AndroidUnavailable(
        "호스트 환경에서는 복사된 오픈프로필 링크를 읽을 수 없습니다".to_string(),
    ))
}

pub fn open_profile_activity(link_id: i64, profile: i32) -> Result<(), NoaError> {
    if link_id <= 0 {
        return Err(NoaError::BadRequest(
            "linkId는 0보다 큰 정수여야 합니다".to_string(),
        ));
    }
    if focused_app().is_some_and(|value| value.contains("OlkOpenProfileViewerActivity")) {
        run("/system/bin/input", &["keyevent", "BACK"])?;
        let _ = wait_for_focus_change("OlkOpenProfileViewerActivity", Duration::from_secs(2));
    }
    let user = profile.to_string();
    let link = link_id.to_string();
    run(
        "/system/bin/am",
        &[
            "start",
            "--user",
            &user,
            "-f",
            "335544320",
            "-n",
            OPEN_PROFILE_ACTIVITY,
            "--el",
            "request_openlink_id",
            &link,
            "--es",
            "extra_call_type",
            "COMMON",
            "--es",
            "referer",
            "noa",
        ],
    )?;
    #[cfg(not(target_os = "android"))]
    if !wait_for_focus("OlkOpenProfileViewerActivity", Duration::from_secs(10)) {
        return Err(NoaError::AndroidUnavailable(
            "오픈프로필 화면을 열지 못했습니다".to_string(),
        ));
    }

    #[cfg(target_os = "android")]
    {
        if !super::ui_agent::wait_open_profile_more().map_err(NoaError::AndroidUnavailable)? {
            return Err(NoaError::AndroidUnavailable(
                "오픈프로필 공유 메뉴를 찾지 못했습니다".to_string(),
            ));
        }
    }
    Ok(())
}

pub fn leave_chat(room_id: i64, profile: i32, expected_room_name: &str) -> Result<(), NoaError> {
    match leave_chat_once(room_id, profile, expected_room_name) {
        Err(NoaError::AndroidUnavailable(message))
            if message == "채팅방 나가기 확인 버튼을 찾지 못했습니다" =>
        {
            tracing::warn!(room_id, "나가기 확인 화면을 다시 엽니다");
            leave_chat_once(room_id, profile, expected_room_name)
        }
        result => result,
    }
}

fn leave_chat_once(room_id: i64, profile: i32, expected_room_name: &str) -> Result<(), NoaError> {
    open_member_info(room_id, profile, expected_room_name)?;
    let settings_button = if fast_click_settings() {
        None
    } else {
        let settings_button = wait_for(Duration::from_secs(5), |nodes| {
            nodes
                .iter()
                .find(|node| matches_label(node, &SETTINGS_LABELS))
                .and_then(|node| node.bounds)
        })
        .ok_or_else(|| {
            NoaError::AndroidUnavailable("채팅방 설정 버튼을 찾지 못했습니다".to_string())
        })?;
        tap_labeled(settings_button, &SETTINGS_LABELS)?;
        Some(settings_button)
    };
    if let Some(settings_button) = settings_button {
        if !wait_for_focus(
            "OpenChatRoomInformationActivity",
            Duration::from_millis(750),
        ) {
            input_tap(settings_button)?;
        }
        if !wait_for_focus("OpenChatRoomInformationActivity", Duration::from_secs(5)) {
            return Err(NoaError::AndroidUnavailable(
                "채팅방 설정 화면을 열지 못했습니다".to_string(),
            ));
        }
    }

    let leave_button = if fast_scroll_click_leave_chatroom() {
        None
    } else {
        if !wait_for_focus("OpenChatRoomInformationActivity", Duration::from_secs(5)) {
            return Err(NoaError::AndroidUnavailable(
                "채팅방 설정 화면을 열지 못했습니다".to_string(),
            ));
        }
        // 최초 3회 x 150ms = 입력 시간 약 450ms로 하단 설정까지 먼저 이동 test29
        let leave_button = find_with_scroll(Duration::from_secs(12), 3, |nodes| {
            nodes
                .iter()
                .find(|node| {
                    node.resource_id == SETTING_BUTTON_ID
                        && matches_label(node, &LEAVE_CHATROOM_LABELS)
                })
                .and_then(|node| node.bounds)
        })
        .ok_or_else(|| {
            NoaError::AndroidUnavailable("채팅방 나가기 항목을 찾지 못했습니다".to_string())
        })?;
        tap_labeled(leave_button, &LEAVE_CHATROOM_LABELS)?;
        Some(leave_button)
    };

    let confirmation = if fast_click_leave_confirmation() {
        None
    } else {
        let mut confirmation = wait_for(Duration::from_millis(750), select_leave_confirmation);
        if confirmation.is_none()
            && let Some(leave_button) = leave_button
        {
            input_tap(leave_button)?;
            confirmation = wait_for(Duration::from_secs(8), select_leave_confirmation);
        }
        let confirmation = confirmation.ok_or_else(|| {
            NoaError::AndroidUnavailable("채팅방 나가기 확인 버튼을 찾지 못했습니다".to_string())
        })?;
        tap_labeled(confirmation, &LEAVE_LABELS)?;
        Some(confirmation)
    };

    if !fast_wait_for_leave_completed() {
        if !wait_for_focus_change(
            "OpenChatRoomInformationActivity",
            Duration::from_millis(750),
        ) && let Some(confirmation) = confirmation
        {
            input_tap(confirmation)?;
        }
        if !wait_for_focus_change("OpenChatRoomInformationActivity", Duration::from_secs(10)) {
            return Err(NoaError::AndroidUnavailable(
                "채팅방 나가기 완료를 확인하지 못했습니다".to_string(),
            ));
        }
    }
    restart_kakao_chat_list(profile)
}

fn fast_click_settings() -> bool {
    #[cfg(target_os = "android")]
    match super::ui_agent::click_settings() {
        Ok(clicked) => return clicked,
        Err(error) => {
            tracing::warn!(%error, "채팅방 설정 빠른 선택 실패, 화면 덤프로 전환합니다");
        }
    }
    false
}

fn fast_scroll_click_leave_chatroom() -> bool {
    #[cfg(target_os = "android")]
    match super::ui_agent::scroll_click_leave_chatroom() {
        Ok(clicked) => return clicked,
        Err(error) => {
            tracing::warn!(%error, "채팅방 나가기 빠른 선택 실패, 화면 덤프로 전환합니다");
        }
    }
    false
}

fn fast_click_leave_confirmation() -> bool {
    #[cfg(target_os = "android")]
    match super::ui_agent::click_leave_confirmation() {
        Ok(clicked) => return clicked,
        Err(error) => {
            tracing::warn!(%error, "채팅방 나가기 확인 빠른 선택 실패, 화면 덤프로 전환합니다");
        }
    }
    false
}

fn fast_wait_for_leave_completed() -> bool {
    #[cfg(target_os = "android")]
    match super::ui_agent::wait_for_resource_gone(SETTING_BUTTON_ID, Duration::from_secs(10)) {
        Ok(completed) => return completed,
        Err(error) => {
            tracing::warn!(%error, "채팅방 나가기 완료 빠른 검증 실패, 포커스 검증으로 전환합니다");
        }
    }
    false
}

fn select_leave_confirmation(nodes: &[UiNode]) -> Option<Bounds> {
    nodes
        .iter()
        .find(|node| {
            node.clickable
                && node.class_name.ends_with("Button")
                && matches_label(node, &LEAVE_LABELS)
        })
        .and_then(|node| node.bounds)
}

pub fn kick_member(
    room_id: i64,
    profile: i32,
    room_name: &str,
    nickname: &str,
) -> Result<(), NoaError> {
    open_member_info(room_id, profile, room_name)?;
    select_member_profile(nickname)?;

    let action_clicked = {
        #[cfg(target_os = "android")]
        {
            match super::ui_agent::click_kick_profile(nickname) {
                Ok(true) => true,
                Ok(false) => {
                    return Err(NoaError::AndroidUnavailable(
                        "강퇴 권한이 없거나 해당 참여자를 강퇴할 수 없습니다".to_string(),
                    ));
                }
                Err(error) => {
                    tracing::warn!(%error, "프로필 강퇴 버튼 빠른 선택 실패, 화면 덤프로 전환합니다");
                    false
                }
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            false
        }
    };
    if !action_clicked {
        let action = wait_for(Duration::from_secs(8), |nodes| {
            // 프로필 이름과 실제 kick label을 함께 확인해 다른 사용자를 누르는 것을 방지한다.
            let correct_profile = nodes.iter().any(|node| {
                node.resource_id == "com.kakao.talk.openlink:id/name"
                    && node.text.trim() == nickname
            });
            correct_profile.then(|| {
                nodes
                    .iter()
                    .find(|node| matches_label(node, &KICK_MEMBER_LABELS))
                    .and_then(|node| node.bounds)
            })?
        })
        .ok_or_else(|| {
            NoaError::AndroidUnavailable(
                "강퇴 권한이 없거나 해당 참여자를 강퇴할 수 없습니다".to_string(),
            )
        })?;
        tap_labeled(action, &KICK_MEMBER_LABELS)?;
    }

    #[cfg(target_os = "android")]
    match super::ui_agent::click_kick_confirmation() {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, "강퇴 확인 빠른 선택 실패, 화면 덤프로 전환합니다");
        }
    }

    let confirmation = wait_for(Duration::from_secs(8), |nodes| {
        nodes
            .iter()
            .find(|node| {
                node.clickable
                    && node.class_name.ends_with("Button")
                    && matches_label(node, &REMOVE_LABELS)
            })
            .and_then(|node| node.bounds)
    })
    .ok_or_else(|| NoaError::AndroidUnavailable("강퇴 확인 버튼을 찾지 못했습니다".to_string()))?;
    tap_labeled(confirmation, &REMOVE_LABELS)?;
    // 실제 성공 여부는 호출자가 KakaoTalk DB의 active_member_ids로 검증한다.
    Ok(())
}

fn select_member_profile(nickname: &str) -> Result<(), NoaError> {
    #[cfg(target_os = "android")]
    match super::ui_agent::expand_member_list() {
        Ok(true) => {}
        Ok(false) => tracing::debug!("멤버 목록 펼치기가 없어 현재 화면에서 참여자를 찾습니다"),
        Err(error) => {
            tracing::warn!(%error, "멤버 목록 펼치기 실패, 현재 화면 탐색으로 전환합니다")
        }
    }

    #[cfg(target_os = "android")]
    match super::ui_agent::scroll_click_text(nickname) {
        Ok(super::ui_agent::TextClickResult::Clicked) => return Ok(()),
        Ok(super::ui_agent::TextClickResult::NotFound) => {
            return Err(NoaError::NotFound(format!(
                "참여자를 찾지 못했습니다: {nickname}"
            )));
        }
        Ok(super::ui_agent::TextClickResult::Ambiguous) => {
            return Err(NoaError::BadRequest(format!(
                "같은 닉네임의 참여자가 여러 명입니다: {nickname}"
            )));
        }
        Err(error) => {
            tracing::warn!(%error, "참여자 빠른 스크롤 선택 실패, 화면 덤프로 전환합니다");
        }
    }

    let display = display_bounds()?;
    let member = find_participant(display, nickname, Duration::from_secs(20))?;
    tap_named(member, nickname)
}

fn open_member_info(room_id: i64, profile: i32, expected_room_name: &str) -> Result<(), NoaError> {
    let user = profile.to_string();
    let room = room_id.to_string();
    run(
        "/system/bin/am",
        &[
            "start",
            "--user",
            &user,
            "-f",
            "335544320",
            "-n",
            MEMBER_INFO_ACTIVITY,
            "--el",
            "chatId",
            &room,
        ],
    )?;
    if !wait_for_focus("ChatRoomSideActivity", Duration::from_secs(8)) {
        return Err(NoaError::AndroidUnavailable(
            "멤버 정보 Activity를 열지 못했습니다".to_string(),
        ));
    }
    #[cfg(target_os = "android")]
    match super::ui_agent::wait_for_text(expected_room_name, Duration::from_secs(8)) {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(error) => {
            tracing::warn!(%error, "멤버 정보 화면 빠른 검증 실패, 화면 덤프로 전환합니다");
        }
    }
    wait_for(Duration::from_secs(8), |nodes| {
        member_info_matches(nodes, expected_room_name).then_some(())
    })
    .ok_or_else(|| {
        NoaError::AndroidUnavailable(format!(
            "다른 채팅방의 멤버 정보 화면이 열렸습니다: {expected_room_name}"
        ))
    })
}

fn member_info_matches(nodes: &[UiNode], expected_room_name: &str) -> bool {
    nodes
        .iter()
        .any(|node| node.class_name.ends_with("TextView") && node.text.trim() == expected_room_name)
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
    verify_resend_room(expected_room_name)?;
    let room_verification_ms = stage.elapsed().as_millis();
    let stage = Instant::now();
    click_matching_resend_indicator(message, attachment)?;
    let indicator_click_ms = stage.elapsed().as_millis();
    let stage = Instant::now();

    #[cfg(target_os = "android")]
    let confirmation_mode = match super::ui_agent::click_resend_confirmation() {
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
    };
    #[cfg(not(target_os = "android"))]
    let confirmation_mode = "xml";

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
        indicator_click_ms,
        confirmation_ms = stage.elapsed().as_millis(),
        total_ms = started.elapsed().as_millis(),
        confirmation_mode,
        "custom 접근성 재전송 단계별 소요시간"
    );
    Ok(())
}

fn verify_resend_room(expected_room_name: &str) -> Result<(), NoaError> {
    #[cfg(target_os = "android")]
    match super::ui_agent::wait_for_resource_text(
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

fn click_matching_resend_indicator(message: &str, attachment: &str) -> Result<(), NoaError> {
    #[cfg(target_os = "android")]
    let mut fallback_timeout = Duration::from_secs(15);
    #[cfg(not(target_os = "android"))]
    let fallback_timeout = Duration::from_secs(15);
    #[cfg(target_os = "android")]
    match super::ui_agent::click_resend_target(
        &target_strings(message, attachment),
        Duration::from_secs(15),
    ) {
        Ok(super::ui_agent::ResendTargetClickResult::Clicked) => return Ok(()),
        Ok(super::ui_agent::ResendTargetClickResult::NotFound) => {
            tracing::warn!(
                "대상 메시지 재전송 표시를 빠른 탐색에서 찾지 못해 화면 덤프로 검증합니다"
            );
            fallback_timeout = Duration::from_secs(3);
        }
        Ok(super::ui_agent::ResendTargetClickResult::Ambiguous) => {
            tracing::warn!("재전송 표시가 여러 개라 화면 덤프로 대상 버블을 검증합니다");
            fallback_timeout = Duration::from_secs(3);
        }
        Err(error) => {
            tracing::warn!(%error, "재전송 대상 빠른 선택 실패, 화면 덤프로 전환합니다");
        }
    }

    let indicator = wait_for(fallback_timeout, |nodes| {
        select_indicator(nodes, message, attachment)
    })
    .ok_or_else(|| {
        NoaError::AndroidUnavailable(
            "대상 메시지와 일치하는 재전송 표시를 찾지 못했습니다".to_string(),
        )
    })?;
    tap_resend_indicator(indicator)
}

fn tap_resend_indicator(bounds: Bounds) -> Result<(), NoaError> {
    #[cfg(target_os = "android")]
    match super::ui_agent::click_resource_at(RESEND_INDICATOR_ID, bounds) {
        Ok(true) => return Ok(()),
        Ok(false) => {
            return Err(NoaError::AndroidUnavailable(
                "클릭 직전에 재전송 표시가 사라졌습니다".to_string(),
            ));
        }
        Err(error) => {
            tracing::warn!(%error, "재전송 표시 의미 기반 클릭 실패, 좌표 폴백을 사용합니다");
        }
    }
    tap(bounds)
}

fn wait_for<T>(timeout: Duration, select: impl Fn(&[UiNode]) -> Option<T>) -> Option<T> {
    // 측정값 37ms dump + 16ms park = 실패 반복 1회당 대략 53ms test32
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(nodes) = dump_nodes()
            && let Some(value) = select(&nodes)
        {
            return Some(value);
        }
        if !wait_for_next_probe(deadline) {
            return None;
        }
    }
}

fn wait_for_open_chat_destination(timeout: Duration) -> Result<OpenChatDestination, NoaError> {
    #[cfg(target_os = "android")]
    match super::ui_agent::wait_open_chat_destination(timeout) {
        Ok(Some(super::ui_agent::OpenChatDestination::Entered(title))) => {
            return Ok(OpenChatDestination::Entered(title));
        }
        Ok(Some(super::ui_agent::OpenChatDestination::Cover(title))) => {
            return Ok(OpenChatDestination::Cover(title, None));
        }
        Ok(Some(super::ui_agent::OpenChatDestination::Rejected(message))) => {
            return Err(open_chat_rejected(message));
        }
        Ok(None) => {
            return Err(NoaError::AndroidUnavailable(
                "오픈채팅 입장 화면을 찾지 못했습니다".to_string(),
            ));
        }
        Err(error) => {
            tracing::warn!(%error, "오픈채팅 화면 빠른 탐색 실패, 화면 덤프로 전환합니다");
        }
    }

    wait_for_open_chat_step(
        timeout,
        |nodes| {
            if let Some(title) = select_chat_title(nodes) {
                Some(OpenChatDestination::Entered(title))
            } else {
                select_open_chat_cover(nodes)
                    .map(|(title, button)| OpenChatDestination::Cover(title, Some(button)))
            }
        },
        "오픈채팅 입장 화면을 찾지 못했습니다",
    )
}

fn wait_for_open_chat_profile(
    selected_profile: &str,
    timeout: Duration,
) -> Result<JoinDestination, NoaError> {
    #[cfg(target_os = "android")]
    match super::ui_agent::wait_open_chat_profile(selected_profile, timeout) {
        Ok(Some(super::ui_agent::OpenChatProfileDestination::Entered(title))) => {
            return Ok(JoinDestination::Entered(title));
        }
        Ok(Some(super::ui_agent::OpenChatProfileDestination::Selected(profile))) => {
            return Ok(JoinDestination::Profile(profile, None));
        }
        Ok(Some(super::ui_agent::OpenChatProfileDestination::Rejected(message))) => {
            return Err(open_chat_rejected(message));
        }
        Ok(Some(super::ui_agent::OpenChatProfileDestination::Ambiguous)) => {
            return Err(NoaError::BadRequest(format!(
                "같은 이름의 오픈채팅 프로필이 여러 개입니다: {selected_profile}"
            )));
        }
        Ok(None) => {
            return Err(NoaError::AndroidUnavailable(format!(
                "선택한 오픈채팅 프로필을 화면에서 찾지 못했습니다: {selected_profile}"
            )));
        }
        Err(error) => {
            tracing::warn!(%error, "오픈채팅 프로필 빠른 선택 실패, 화면 덤프로 전환합니다");
        }
    }

    wait_for_open_chat_step(
        timeout,
        |nodes| {
            if let Some(title) = select_chat_title(nodes) {
                Some(JoinDestination::Entered(title))
            } else if is_profile_picker(nodes) {
                select_profile(nodes, Some(selected_profile))
                    .map(|(name, bounds)| JoinDestination::Profile(name, Some(bounds)))
            } else {
                None
            }
        },
        &format!("선택한 오픈채팅 프로필을 화면에서 찾지 못했습니다: {selected_profile}"),
    )
}

fn wait_for_open_chat_entered(timeout: Duration) -> Result<String, NoaError> {
    #[cfg(target_os = "android")]
    match super::ui_agent::wait_open_chat_entered(timeout) {
        Ok(Some(super::ui_agent::OpenChatDestination::Entered(title))) => return Ok(title),
        Ok(Some(super::ui_agent::OpenChatDestination::Rejected(message))) => {
            return Err(open_chat_rejected(message));
        }
        Ok(Some(super::ui_agent::OpenChatDestination::Cover(_))) => {
            return Err(NoaError::AndroidUnavailable(
                "오픈채팅 참여 화면으로 되돌아갔습니다".to_string(),
            ));
        }
        Ok(None) => {
            return Err(NoaError::AndroidUnavailable(
                "오픈채팅 입장 완료를 확인하지 못했습니다".to_string(),
            ));
        }
        Err(error) => {
            tracing::warn!(%error, "오픈채팅 입장 완료 빠른 검증 실패, 화면 덤프로 전환합니다");
        }
    }

    wait_for_open_chat_step(
        timeout,
        select_chat_title,
        "오픈채팅 입장 완료를 확인하지 못했습니다",
    )
}

fn open_chat_rejected(message: String) -> NoaError {
    NoaError::AndroidUnavailable(format!(
        "KakaoTalk에서 오픈채팅 입장을 거부했습니다: {}",
        message.chars().take(500).collect::<String>()
    ))
}

fn wait_for_open_chat_step<T>(
    timeout: Duration,
    select: impl Fn(&[UiNode]) -> Option<T>,
    timeout_message: &str,
) -> Result<T, NoaError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(nodes) = dump_nodes() {
            if let Some(message) = select_open_chat_rejection(&nodes) {
                return Err(open_chat_rejected(message));
            }
            if let Some(value) = select(&nodes) {
                return Ok(value);
            }
        }
        if !wait_for_next_probe(deadline) {
            return Err(NoaError::AndroidUnavailable(timeout_message.to_string()));
        }
    }
}

fn restart_kakao_chat_list(profile: i32) -> Result<(), NoaError> {
    let user = profile.to_string();
    // 별도 채팅 태스크 종료 뒤 기존 목록 캐시 1개가 남아서 프로세스 재생성으로 즉시 제거 test47
    run(
        "/system/bin/am",
        &[
            "start",
            "--user",
            &user,
            "-S",
            "-n",
            "com.kakao.talk/.activity.SplashActivity",
        ],
    )
}

fn find_with_scroll<T>(
    timeout: Duration,
    initial_swipes: usize,
    select: impl Fn(&[UiNode]) -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + timeout;
    // 첫 화면은 지정 횟수만큼 내리고 그 뒤부터 1회씩 내려서 3 1 1 순서인지 확인 test33
    let mut pending_swipes = Some(initial_swipes);
    loop {
        if let Ok(nodes) = dump_nodes() {
            if let Some(value) = select(&nodes) {
                return Some(value);
            }
            if let Some(bounds) = nodes
                .iter()
                .find(|node| {
                    node.resource_id == "com.kakao.talk:id/recycler_view" && node.scrollable
                })
                .and_then(|node| node.bounds)
            {
                let swipes = pending_swipes.take().unwrap_or(1);
                for _ in 0..swipes {
                    let _ = swipe_up(bounds);
                }
            }
        }
        if !wait_for_next_probe(deadline) {
            return None;
        }
    }
}

fn find_participant(
    display: Bounds,
    nickname: &str,
    timeout: Duration,
) -> Result<Bounds, NoaError> {
    let deadline = Instant::now() + timeout;
    let mut participant_section = false;
    loop {
        if let Ok(nodes) = dump_nodes() {
            participant_section |= nodes.iter().any(|node| {
                matches_label(node, &PARTICIPANTS_LABELS) || matches_label(node, &SELF_LABELS)
            });
            if participant_section {
                let mut candidates: Vec<Bounds> = nodes
                    .iter()
                    .filter(|node| {
                        node.resource_id.is_empty()
                            && node.class_name.ends_with("TextView")
                            && node.text.trim() == nickname
                    })
                    .filter_map(|node| node.bounds)
                    .collect();
                candidates.sort_unstable();
                candidates.dedup();
                // 정렬 후 중복 제거해서 0개 실패 1개 성공 2개 이상 거부로 3분기 test34
                match candidates.as_slice() {
                    [bounds] => return Ok(*bounds),
                    [_, _, ..] => {
                        return Err(NoaError::BadRequest(format!(
                            "같은 닉네임의 참여자가 여러 명입니다: {nickname}"
                        )));
                    }
                    [] => {}
                }
            }
            let _ = swipe_up(display);
        }
        if !wait_for_next_probe(deadline) {
            return Err(NoaError::NotFound(format!(
                "강퇴할 참여자를 찾지 못했습니다: {nickname}"
            )));
        }
    }
}

fn dump_nodes() -> Result<Vec<UiNode>, NoaError> {
    // 상주 dump 37ms와 CLI dump 1960ms면 1960 / 37 = 약 53배 차이 test35
    #[cfg(target_os = "android")]
    if let Err(error) = super::ui_agent::dump_hierarchy() {
        tracing::warn!(%error, "상주 UiAutomator 에이전트 사용 실패, CLI dump로 전환합니다");
        run(
            "/system/bin/uiautomator",
            &["dump", "--compressed", UI_DUMP],
        )?;
    }
    #[cfg(not(target_os = "android"))]
    run(
        "/system/bin/uiautomator",
        &["dump", "--compressed", UI_DUMP],
    )?;
    let xml = fs::read_to_string(UI_DUMP).map_err(|error| {
        NoaError::AndroidUnavailable(format!("접근성 화면 구조를 읽지 못했습니다: {error}"))
    })?;
    Ok(parse_nodes(&xml))
}

fn tap(bounds: (i32, i32, i32, i32)) -> Result<(), NoaError> {
    #[cfg(target_os = "android")]
    {
        // x는 left + right를 2로 나누고 y도 top + bottom을 2로 나눠 정중앙 1점 계산 test36
        let x = (bounds.0 + bounds.2) / 2;
        let y = (bounds.1 + bounds.3) / 2;
        if super::ui_agent::click(x, y).is_ok() {
            return Ok(());
        }
    }
    // 상주 에이전트 재시작에도 실패한 경우에만 input 1회로 복구 시도 test46
    input_tap(bounds)
}

fn input_tap(bounds: Bounds) -> Result<(), NoaError> {
    let x = (bounds.0 + bounds.2) / 2;
    let y = (bounds.1 + bounds.3) / 2;
    run(
        "/system/bin/input",
        &["tap", &x.to_string(), &y.to_string()],
    )
}

fn tap_labeled(bounds: Bounds, labels: &[&str]) -> Result<(), NoaError> {
    #[cfg(not(target_os = "android"))]
    let _ = labels;
    #[cfg(target_os = "android")]
    for label in labels {
        if super::ui_agent::click_label(label).is_ok() {
            return Ok(());
        }
    }
    tap(bounds)
}

fn tap_named(bounds: Bounds, text: &str) -> Result<(), NoaError> {
    #[cfg(not(target_os = "android"))]
    let _ = text;
    #[cfg(target_os = "android")]
    if super::ui_agent::click_text_at(text, bounds).is_ok() {
        return Ok(());
    }
    tap(bounds)
}

fn swipe_up(bounds: (i32, i32, i32, i32)) -> Result<(), NoaError> {
    // 아래 80퍼센트에서 위 20퍼센트로 움직여 전체 높이의 60퍼센트를 150ms에 이동 test37
    let x = ((bounds.0 + bounds.2) / 2).to_string();
    let top = (bounds.1 + (bounds.3 - bounds.1) / 5).to_string();
    let bottom = (bounds.3 - (bounds.3 - bounds.1) / 5).to_string();
    run(
        "/system/bin/input",
        &["swipe", &x, &bottom, &x, &top, "150"],
    )
}

fn display_bounds() -> Result<Bounds, NoaError> {
    let output = command_text("/system/bin/wm", &["size"])?;
    let value = output
        .lines()
        .rev()
        .find_map(|line| line.split_once(':').map(|value| value.1.trim()))
        .and_then(|value| value.split_once('x'))
        .ok_or_else(|| {
            NoaError::AndroidUnavailable("Android 화면 크기를 확인하지 못했습니다".to_string())
        })?;
    let width = value.0.parse::<i32>().map_err(|_| {
        NoaError::AndroidUnavailable("Android 화면 너비를 확인하지 못했습니다".to_string())
    })?;
    let height = value.1.parse::<i32>().map_err(|_| {
        NoaError::AndroidUnavailable("Android 화면 높이를 확인하지 못했습니다".to_string())
    })?;
    Ok((0, 0, width, height))
}

fn wait_for_focus_change(activity: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if focused_app().is_some_and(|value| !value.contains(activity)) {
            return true;
        }
        if !wait_for_next_probe(deadline) {
            return false;
        }
    }
}

fn wait_for_focus(activity: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if focused_app().is_some_and(|value| value.contains(activity)) {
            return true;
        }
        if !wait_for_next_probe(deadline) {
            return false;
        }
    }
}

fn wait_for_member_profile_focus(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if focused_app().is_some_and(|value| {
            value.contains("OlkProfileActivity") || value.contains("OlkOpenProfileViewerActivity")
        }) {
            return true;
        }
        if !wait_for_next_probe(deadline) {
            return false;
        }
    }
}

fn wait_for_next_probe(deadline: Instant) -> bool {
    let now = Instant::now();
    if now >= deadline {
        return false;
    }
    // 남은 시간과 16ms 중 작은 값만 기다려 deadline 초과 오차는 최대 16ms 예상 test40
    thread::park_timeout((deadline - now).min(STATE_POLL_INTERVAL));
    true
}

fn focused_app() -> Option<String> {
    command_text("/system/bin/dumpsys", &["window"])
        .ok()?
        .lines()
        .find(|line| line.contains("mFocusedApp="))
        .map(str::trim)
        .map(str::to_string)
}

fn command_text(program: &str, arguments: &[&str]) -> Result<String, NoaError> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| NoaError::AndroidUnavailable(format!("{program} 실행 실패: {error}")))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(NoaError::AndroidUnavailable(format!(
        "{program} 실행 실패: {}",
        if stderr.is_empty() {
            "종료 상태 오류"
        } else {
            &stderr
        }
    )))
}

fn run(program: &str, arguments: &[&str]) -> Result<(), NoaError> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| NoaError::AndroidUnavailable(format!("{program} 실행 실패: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    Err(NoaError::AndroidUnavailable(format!(
        "{program} 실행 실패: {}",
        if detail.is_empty() {
            "종료 상태 오류"
        } else {
            &detail
        }
    )))
}

#[derive(Default)]
struct UiNode {
    resource_id: String,
    class_name: String,
    text: String,
    description: String,
    clickable: bool,
    scrollable: bool,
    bounds: Option<(i32, i32, i32, i32)>,
}

fn parse_nodes(xml: &str) -> Vec<UiNode> {
    // node 태그 1개를 UiNode 1개로 바꿔 결과와 유효 태그 개수가 같은지 확인 test41
    xml.split("<node ")
        .skip(1)
        .filter_map(|part| part.split_once('>').map(|value| value.0))
        .map(|tag| UiNode {
            resource_id: attribute(tag, "resource-id").unwrap_or_default(),
            class_name: attribute(tag, "class").unwrap_or_default(),
            text: attribute(tag, "text").unwrap_or_default(),
            description: attribute(tag, "content-desc").unwrap_or_default(),
            clickable: attribute(tag, "clickable").as_deref() == Some("true"),
            scrollable: attribute(tag, "scrollable").as_deref() == Some("true"),
            bounds: attribute(tag, "bounds").and_then(|value| parse_bounds(&value)),
        })
        .collect()
}

fn open_chat_scheme(value: &str) -> Result<String, NoaError> {
    let parsed = url::Url::parse(value)
        .map_err(|_| NoaError::BadRequest("올바른 오픈채팅 URL이 아닙니다".to_string()))?;
    let mut segments = parsed
        .path_segments()
        .ok_or_else(|| NoaError::BadRequest("올바른 오픈채팅 URL이 아닙니다".to_string()))?;
    let section = segments.next();
    let token = segments.next();
    // 경로는 o 1개 + token 1개 = 정확히 2조각이고 세 번째가 있으면 거부 test42
    let complete = segments.next().is_none();
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("open.kakao.com")
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || section != Some("o")
        || !token.is_some_and(|value| {
            !value.is_empty()
                && value
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        })
        || !complete
    {
        return Err(NoaError::BadRequest(
            "https://open.kakao.com/o/... 형식만 지원합니다".to_string(),
        ));
    }
    Ok(format!("kakaoopen://join?l={}", token.unwrap_or_default()))
}

fn is_open_profile_url(value: &str) -> bool {
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    let Some(mut segments) = parsed.path_segments() else {
        return false;
    };
    let section = segments.next();
    let token = segments.next();
    parsed.scheme() == "https"
        && parsed.host_str() == Some("open.kakao.com")
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && matches!(section, Some("o" | "me"))
        && token.is_some_and(|value| {
            !value.is_empty()
                && value.bytes().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, b'_' | b'-')
                })
        })
        && segments.next().is_none()
}

fn verify_open_profile_url(copied: &str, expected: Option<&str>) -> Result<String, NoaError> {
    let copied = copied.trim();
    if !is_open_profile_url(copied) {
        return Err(NoaError::AndroidUnavailable(
            "클립보드에 복사된 값이 올바른 카카오 오픈링크가 아닙니다".to_string(),
        ));
    }
    if expected.is_some_and(|value| value != copied) {
        return Err(NoaError::AndroidUnavailable(
            "클립보드 링크가 DB의 linkId 링크와 일치하지 않습니다".to_string(),
        ));
    }
    Ok(copied.to_string())
}

fn select_chat_title(nodes: &[UiNode]) -> Option<String> {
    nodes
        .iter()
        .find(|node| node.resource_id == "com.kakao.talk:id/toolbar_default_title_text")
        .map(|node| {
            if node.text.trim().is_empty() {
                node.description.trim().to_string()
            } else {
                node.text.trim().to_string()
            }
        })
        .filter(|value| !value.is_empty())
}

fn select_open_chat_cover(nodes: &[UiNode]) -> Option<(String, (i32, i32, i32, i32))> {
    let title = nodes
        .iter()
        .find(|node| is_open_chat_cover_title_id(&node.resource_id))?
        .text
        .trim()
        .to_string();
    let button = nodes
        .iter()
        .find(|node| {
            node.resource_id == "com.kakao.talk.openlink:id/join_layout"
                || matches_label(node, &JOIN_OPEN_CHAT_LABELS)
        })?
        .bounds?;
    (!title.is_empty()).then_some((title, button))
}

fn select_profile(
    nodes: &[UiNode],
    requested_profile: Option<&str>,
) -> Option<(String, (i32, i32, i32, i32))> {
    if let Some(profile) = requested_profile {
        let profile = profile.trim();
        let mut matches = nodes
            .iter()
            .filter(|node| {
                is_open_chat_profile_name_id(&node.resource_id) && node.text.trim() == profile
            })
            .filter_map(|node| node.bounds);
        let bounds = matches.next()?;
        return matches
            .next()
            .is_none()
            .then(|| (profile.to_string(), bounds));
    }
    nodes
        .iter()
        .find(|node| {
            is_open_chat_profile_name_id(&node.resource_id)
                && !node.text.trim().is_empty()
                && !matches_label(node, &KAKAO_FRIENDS_LABELS)
                && !matches_label(node, &NEW_OPEN_PROFILE_LABELS)
        })
        .and_then(|node| {
            node.bounds
                .map(|bounds| (node.text.trim().to_string(), bounds))
        })
}

fn is_profile_picker(nodes: &[UiNode]) -> bool {
    let cover_is_visible = nodes
        .iter()
        .any(|node| node.resource_id == "com.kakao.talk.openlink:id/join_layout");
    let profile_is_visible = nodes
        .iter()
        .any(|node| is_open_chat_profile_name_id(&node.resource_id));
    !cover_is_visible && profile_is_visible
}

fn is_open_chat_cover_title_id(resource_id: &str) -> bool {
    resource_id == OPEN_CHAT_COVER_TITLE_ID || resource_id.starts_with(OPEN_CHAT_COVER_TITLE_PREFIX)
}

fn is_open_chat_profile_name_id(resource_id: &str) -> bool {
    resource_id == OPEN_CHAT_PROFILE_NAME_ID
        || resource_id.starts_with(OPEN_CHAT_PROFILE_NAME_PREFIX)
}

fn select_open_chat_rejection(nodes: &[UiNode]) -> Option<String> {
    select_open_chat_rejection_message(nodes)
}

fn select_open_chat_rejection_message(nodes: &[UiNode]) -> Option<String> {
    nodes
        .iter()
        .find(|node| node.resource_id == OPEN_CHAT_MESSAGE_ID)
        .map(|node| {
            if node.text.trim().is_empty() {
                node.description.trim()
            } else {
                node.text.trim()
            }
        })
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(500).collect())
}

fn matches_label(node: &UiNode, labels: &[&str]) -> bool {
    // text 1곳 또는 content description 1곳이라 label마다 비교 위치는 2개 test43
    labels
        .iter()
        .any(|label| node.text == *label || node.description == *label)
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let marker = format!("{name}=\"");
    let value = tag.split_once(&marker)?.1.split_once('"')?.0;
    Some(
        value
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

fn parse_bounds(value: &str) -> Option<(i32, i32, i32, i32)> {
    let values: Vec<i32> = value
        .split(['[', ']', ','])
        .filter(|value| !value.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    // 대괄호 2쌍에서 숫자 4개가 나와야 left top right bottom으로 확정 test44
    (values.len() == 4).then(|| (values[0], values[1], values[2], values[3]))
}

fn select_indicator(
    nodes: &[UiNode],
    message: &str,
    attachment: &str,
) -> Option<(i32, i32, i32, i32)> {
    let targets = target_strings(message, attachment);
    let bubbles: Vec<(i32, i32, i32, i32)> = nodes
        .iter()
        .filter(|node| node.resource_id == "com.kakao.talk:id/bubble_linearlayout")
        .filter_map(|node| node.bounds)
        .collect();
    let candidates: Vec<(usize, Bounds)> = nodes
        .iter()
        .filter(|node| node.resource_id == RESEND_INDICATOR_ID)
        .filter_map(|indicator| {
            let bounds = indicator.bounds?;
            let bubble_bounds = bubbles
                .iter()
                .copied()
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
            Some((score, bounds))
        })
        .collect();
    if targets.is_empty() {
        return match candidates.as_slice() {
            [(_, bounds)] => Some(*bounds),
            _ => None,
        };
    }
    candidates
        .into_iter()
        .filter(|(score, _)| *score > 0)
        .max_by_key(|(score, bounds)| (*score, bounds.3))
        .map(|(_, bounds)| bounds)
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
    // 완전 일치는 1000 + 길이, 부분 일치는 최대 100이라 exact 쪽에 최소 10배 가중치 test45
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

fn contains(outer: (i32, i32, i32, i32), inner: (i32, i32, i32, i32)) -> bool {
    outer.0 <= inner.0 && outer.1 <= inner.1 && outer.2 >= inner.2 && outer.3 >= inner.3
}

fn area(bounds: (i32, i32, i32, i32)) -> i64 {
    i64::from(bounds.2 - bounds.0) * i64::from(bounds.3 - bounds.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_every_control_label_in_korean_and_english() {
        let pairs = [
            JOIN_OPEN_CHAT_LABELS,
            SETTINGS_LABELS,
            LEAVE_CHATROOM_LABELS,
            LEAVE_LABELS,
            KICK_MEMBER_LABELS,
            REMOVE_LABELS,
            RESEND_LABELS,
            EXPAND_MEMBER_LABELS,
            COPY_LABELS,
            PARTICIPANTS_LABELS,
            SELF_LABELS,
            KAKAO_FRIENDS_LABELS,
            NEW_OPEN_PROFILE_LABELS,
        ];
        for [korean, english] in pairs {
            assert!(!korean.is_ascii());
            assert!(english.is_ascii());
            assert!(matches_label(
                &UiNode {
                    text: korean.to_string(),
                    ..UiNode::default()
                },
                &[korean, english]
            ));
            assert!(matches_label(
                &UiNode {
                    description: english.to_string(),
                    ..UiNode::default()
                },
                &[korean, english]
            ));
        }
    }

    #[test]
    fn parses_accessibility_nodes() {
        let xml = r#"<hierarchy><node text="" resource-id="com.kakao.talk:id/resend_indicator" class="android.view.View" content-desc="" clickable="false" scrollable="false" bounds="[1,2][31,42]"/><node text="Re-send" resource-id="android:id/button1" class="android.widget.Button" content-desc="" clickable="true" scrollable="false" bounds="[10,20][50,60]"/></hierarchy>"#;
        let nodes = parse_nodes(xml);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].bounds, Some((1, 2, 31, 42)));
        assert_eq!(nodes[1].text, "Re-send");
        assert_eq!(nodes[1].class_name, "android.widget.Button");
        assert!(nodes[1].clickable);
    }

    #[test]
    fn selects_indicator_from_matching_bubble() {
        let xml = r#"<hierarchy><node resource-id="com.kakao.talk:id/bubble_linearlayout" text="" content-desc="" bounds="[0,10][720,300]"><node resource-id="com.kakao.talk:id/resend_indicator" text="" content-desc="Delete or resend failed message" bounds="[100,250][200,290]"/><node resource-id="" text="서울 날씨" content-desc="" bounds="[220,20][700,240]"/></node><node resource-id="com.kakao.talk:id/bubble_linearlayout" text="" content-desc="" bounds="[0,310][720,1000]"><node resource-id="com.kakao.talk:id/resend_indicator" text="" content-desc="Delete or resend failed message" bounds="[100,940][200,990]"/><node resource-id="" text="샵검색" content-desc="" bounds="[220,330][700,900]"/></node></hierarchy>"#;
        let nodes = parse_nodes(xml);
        assert_eq!(
            select_indicator(&nodes, "서울특별시 내일 날씨", r#"{"title":"서울 날씨"}"#),
            Some((100, 250, 200, 290))
        );
        assert_eq!(
            select_indicator(&nodes, "샵검색: #샵검색", r#"{"title":"샵검색"}"#),
            Some((100, 940, 200, 990))
        );
        assert_eq!(
            select_indicator(&nodes, "일치하지 않는 메시지", r#"{"title":"다른 값"}"#),
            None
        );
        assert_eq!(select_indicator(&nodes, "", "{}"), None);

        let single = parse_nodes(
            r#"<hierarchy><node resource-id="com.kakao.talk:id/bubble_linearlayout" text="" content-desc="" bounds="[0,10][720,300]"><node resource-id="com.kakao.talk:id/resend_indicator" text="" content-desc="Delete or resend failed message" bounds="[100,250][200,290]"/></node></hierarchy>"#,
        );
        assert_eq!(
            select_indicator(&single, "", "{}"),
            Some((100, 250, 200, 290))
        );
    }

    #[test]
    fn builds_native_open_chat_links() {
        assert_eq!(
            open_chat_scheme("https://open.kakao.com/o/ggshw4Ai").unwrap(),
            "kakaoopen://join?l=ggshw4Ai"
        );
        assert!(open_chat_scheme("http://open.kakao.com/o/ggshw4Ai").is_err());
        assert!(open_chat_scheme("https://example.com/o/ggshw4Ai").is_err());
        assert!(open_chat_scheme("https://open.kakao.com/not-open/ggshw4Ai").is_err());
        assert!(open_chat_scheme("https://open.kakao.com/o/a/b").is_err());
        assert!(open_chat_scheme("https://open.kakao.com/o/ggshw4Ai?q=1").is_err());
        assert!(open_chat_scheme("https://open.kakao.com/o/ggshw4Ai#fragment").is_err());
    }

    #[test]
    fn verifies_the_copied_open_profile_url() {
        let url = "https://open.kakao.com/o/ggshw4Ai";
        assert_eq!(verify_open_profile_url(url, None).unwrap(), url);
        assert_eq!(verify_open_profile_url(url, Some(url)).unwrap(), url);
        let profile_url = "https://open.kakao.com/me/profile_123";
        assert_eq!(
            verify_open_profile_url(profile_url, None).unwrap(),
            profile_url
        );
        assert!(verify_open_profile_url(url, Some("https://open.kakao.com/o/another")).is_err());
        assert!(verify_open_profile_url("https://example.com/o/ggshw4Ai", None).is_err());
    }

    #[test]
    fn matches_the_direct_member_info_room_title() {
        let nodes = parse_nodes(
            r#"<hierarchy><node text="Jordy Test2" resource-id="" class="android.widget.TextView" content-desc="" clickable="false" scrollable="false" bounds="[56,384][664,442]"/><node text="Participants" resource-id="" class="android.widget.TextView" content-desc="" clickable="false" scrollable="false" bounds="[54,716][206,749]"/></hierarchy>"#,
        );
        assert!(member_info_matches(&nodes, "Jordy Test2"));
        assert!(!member_info_matches(&nodes, "다른 채팅방"));
    }

    #[test]
    fn selects_requested_or_first_open_profile() {
        let xml = r#"<hierarchy><node text="" resource-id="" class="android.widget.Button" content-desc="지민, Set as My Profile" clickable="true" scrollable="false" bounds="[0,100][720,200]"/><node text="지민" resource-id="com.kakao.talk.openlink:id/profile_name_res_0x85060182" class="android.widget.TextView" content-desc="" clickable="false" scrollable="false" bounds="[120,120][200,180]"/><node text="Kakao Friends" resource-id="com.kakao.talk.openlink:id/profile_name_res_0x85060182" class="android.widget.TextView" content-desc="" clickable="false" scrollable="false" bounds="[120,220][300,280]"/></hierarchy>"#;
        let nodes = parse_nodes(xml);
        assert_eq!(
            select_profile(&nodes, Some("지민")),
            Some(("지민".to_string(), (120, 120, 200, 180)))
        );
        assert_eq!(
            select_profile(&nodes, None),
            Some(("지민".to_string(), (120, 120, 200, 180)))
        );
        assert!(select_profile(&nodes, Some("없는 프로필")).is_none());
        let duplicated = parse_nodes(
            r#"<hierarchy><node text="지민" resource-id="com.kakao.talk.openlink:id/profile_name" class="android.widget.TextView" content-desc="" clickable="false" scrollable="false" bounds="[120,120][200,180]"/><node text="지민" resource-id="com.kakao.talk.openlink:id/profile_name" class="android.widget.TextView" content-desc="" clickable="false" scrollable="false" bounds="[120,220][200,280]"/></hierarchy>"#,
        );
        assert!(select_profile(&duplicated, Some("지민")).is_none());
        assert!(is_profile_picker(&nodes));
    }

    #[test]
    fn selects_open_chat_cover_and_entered_room() {
        let cover = parse_nodes(
            r#"<hierarchy><node text="Jordy Test2" resource-id="com.kakao.talk.openlink:id/title_res_0x850601db" class="android.widget.TextView" content-desc="" clickable="false" scrollable="false" bounds="[40,578][566,643]"/><node text="" resource-id="com.kakao.talk.openlink:id/join_layout" class="android.widget.Button" content-desc="Join Open Chat" clickable="true" scrollable="false" bounds="[0,1088][720,1184]"/></hierarchy>"#,
        );
        assert_eq!(
            select_open_chat_cover(&cover),
            Some(("Jordy Test2".to_string(), (0, 1088, 720, 1184)))
        );
        assert!(!is_profile_picker(&cover));
        let entered = parse_nodes(
            r#"<hierarchy><node text="" resource-id="com.kakao.talk:id/toolbar_default_title_text" class="android.widget.TextView" content-desc="Jordy Test2" clickable="false" scrollable="false" bounds="[112,70][271,105]"/></hierarchy>"#,
        );
        assert_eq!(select_chat_title(&entered).as_deref(), Some("Jordy Test2"));

        let rejected = parse_nodes(
            r#"<hierarchy><node text="You cannot join this chatroom." resource-id="com.kakao.talk:id/txt_message" class="android.widget.TextView" content-desc="" clickable="false" scrollable="false" bounds="[128,490][592,633]"/></hierarchy>"#,
        );
        assert_eq!(
            select_open_chat_rejection_message(&rejected).as_deref(),
            Some("You cannot join this chatroom.")
        );
    }
}
