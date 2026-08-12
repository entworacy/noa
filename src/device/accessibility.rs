use std::{
    fs,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use crate::failure::NoaError;

// dump 파일 1개를 계속 덮어써서 화면당 임시 파일 증가는 0개로 확인 test24
const UI_DUMP: &str = "/data/local/tmp/noa/custom-accessibility.xml";
// 1000 / 16 = 초당 최대 62회 확인이고 실제 37ms dump까지 합치면 초당 약 18회 예상 test25
const STATE_POLL_INTERVAL: Duration = Duration::from_millis(16);
// APK 26 6 3의 ko en split에서 확인한 12쌍이라 총 문자열은 24개로 계산 test26
type LabelPair = [&'static str; 2];
const JOIN_OPEN_CHAT_LABELS: LabelPair = ["오픈채팅 참여하기", "Join Open Chat"];
const MORE_LABELS: LabelPair = ["더보기", "More"];
const SETTINGS_LABELS: LabelPair = ["설정", "Settings"];
const LEAVE_CHATROOM_LABELS: LabelPair = ["채팅방 나가기", "Leave chatroom"];
const LEAVE_LABELS: LabelPair = ["나가기", "Leave"];
const KICK_MEMBER_LABELS: LabelPair = ["대화상대 내보내기", "Send out participant"];
const REMOVE_LABELS: LabelPair = ["내보내기", "Remove"];
const RESEND_LABELS: LabelPair = ["재전송", "Re-send"];
const PARTICIPANTS_LABELS: LabelPair = ["대화상대", "Participants"];
const SELF_LABELS: LabelPair = ["나", "me"];
const KAKAO_FRIENDS_LABELS: LabelPair = ["카카오프렌즈", "Kakao Friends"];
const NEW_OPEN_PROFILE_LABELS: LabelPair = ["새 오픈프로필", "New Open Profile"];

const KAKAO_PACKAGE: &str = "com.kakao.talk";
const CHAT_ACTIVITY: &str = "com.kakao.talk/.activity.RecentExcludeIntentFilterActivity";
const CHAT_ACTION: &str = "com.kakao.talk.intent.action.ENTER_CHAT_ROOM";
// left top right bottom 순서의 좌표 4개로 클릭 영역 1개 표현 test27
type Bounds = (i32, i32, i32, i32);

enum JoinDestination {
    Entered(String),
    Profile(String, Bounds),
}

enum OpenChatDestination {
    Entered(String),
    Cover(String, Bounds),
}

pub fn join_open_chat(
    url: &str,
    profile: i32,
    requested_profile: Option<&str>,
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
            "-W",
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
    tap(join_button)?;
    let destination = wait_for(Duration::from_secs(12), |nodes| {
        if chat_room_is_focused() {
            select_chat_title(nodes).map(JoinDestination::Entered)
        } else if is_profile_picker(nodes) {
            select_profile(nodes, requested_profile)
                .map(|(name, bounds)| JoinDestination::Profile(name, bounds))
        } else {
            None
        }
    })
    .ok_or_else(|| {
        requested_profile
            .map(|value| {
                NoaError::AndroidUnavailable(format!(
                    "요청한 오픈채팅 프로필을 찾지 못했습니다: {value}"
                ))
            })
            .unwrap_or_else(|| {
                NoaError::AndroidUnavailable(
                    "오픈채팅 입장 또는 프로필 선택 화면을 찾지 못했습니다".to_string(),
                )
            })
    })?;
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
    tap(profile_button)?;
    let entered_room = wait_for(Duration::from_secs(20), |nodes| {
        chat_room_is_focused().then(|| select_chat_title(nodes))?
    })
    .ok_or_else(|| {
        NoaError::AndroidUnavailable("오픈채팅 입장 완료를 확인하지 못했습니다".to_string())
    })?;
    if entered_room != room_name {
        return Err(NoaError::AndroidUnavailable(format!(
            "다른 채팅방이 열렸습니다: {entered_room}"
        )));
    }
    Ok((room_name, Some(selected_profile)))
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
    open_room_side(room_id, profile, Some(expected_room_name))?;
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
    // 최초 3회 x 150ms = 입력 시간 약 450ms로 하단 설정까지 먼저 이동 test29
    let leave_button = find_with_scroll(Duration::from_secs(12), 3, |nodes| {
        nodes
            .iter()
            .find(|node| {
                node.resource_id == "com.kakao.talk:id/setting_button"
                    && matches_label(node, &LEAVE_CHATROOM_LABELS)
            })
            .and_then(|node| node.bounds)
    })
    .ok_or_else(|| {
        NoaError::AndroidUnavailable("채팅방 나가기 항목을 찾지 못했습니다".to_string())
    })?;
    tap(leave_button)?;
    let mut confirmation = wait_for(Duration::from_millis(750), select_leave_confirmation);
    if confirmation.is_none() {
        input_tap(leave_button)?;
        confirmation = wait_for(Duration::from_secs(8), select_leave_confirmation);
    }
    let confirmation = confirmation.ok_or_else(|| {
        NoaError::AndroidUnavailable("채팅방 나가기 확인 버튼을 찾지 못했습니다".to_string())
    })?;
    tap(confirmation)?;
    if !wait_for_focus_change(
        "OpenChatRoomInformationActivity",
        Duration::from_millis(750),
    ) {
        input_tap(confirmation)?;
    }
    if !wait_for_focus_change("OpenChatRoomInformationActivity", Duration::from_secs(10)) {
        return Err(NoaError::AndroidUnavailable(
            "채팅방 나가기 완료를 확인하지 못했습니다".to_string(),
        ));
    }
    restart_kakao_chat_list(profile)
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
    let display = open_room_side(room_id, profile, Some(room_name))?;
    let member = find_participant(display, nickname, Duration::from_secs(20))?;
    tap(member)?;
    if !wait_for_focus("OlkProfileActivity", Duration::from_secs(8)) {
        return Err(NoaError::AndroidUnavailable(
            "강퇴 대상의 오픈프로필을 열지 못했습니다".to_string(),
        ));
    }
    let action = wait_for(Duration::from_secs(8), |nodes| {
        // profile name 1회와 실제 kick label 1회를 모두 맞춰야 해서 조건은 2단계 test31
        let correct_profile = nodes.iter().any(|node| {
            node.resource_id == "com.kakao.talk.openlink:id/name" && node.text.trim() == nickname
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
    tap(action)?;
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
    tap(confirmation)?;
    if wait_for_focus_change("OlkProfileActivity", Duration::from_secs(12)) {
        Ok(())
    } else {
        Err(NoaError::AndroidUnavailable(
            "강퇴 완료를 확인하지 못했습니다".to_string(),
        ))
    }
}

fn open_room_side(
    room_id: i64,
    profile: i32,
    expected_room_name: Option<&str>,
) -> Result<Bounds, NoaError> {
    if !current_chat_matches(expected_room_name) {
        let user = profile.to_string();
        let room = room_id.to_string();
        run(
            "/system/bin/am",
            &[
                "start",
                "--user",
                &user,
                "-W",
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
        wait_for_ui_idle();
    }
    wait_for(Duration::from_secs(8), |nodes| {
        let title = chat_room_is_focused().then(|| select_chat_title(nodes))??;
        room_title_matches(&title, expected_room_name).then_some(())
    })
    .ok_or_else(|| {
        let expected = expected_room_name
            .map(|name| format!(": {name}"))
            .unwrap_or_default();
        NoaError::AndroidUnavailable(format!("대상 채팅방 화면을 열지 못했습니다{expected}"))
    })?;
    let display = display_bounds()?;
    let more_button = wait_for(Duration::from_secs(5), |nodes| {
        nodes
            .iter()
            .find(|node| matches_label(node, &MORE_LABELS))
            .and_then(|node| node.bounds)
    })
    .ok_or_else(|| {
        NoaError::AndroidUnavailable("채팅방 더보기 버튼을 찾지 못했습니다".to_string())
    })?;
    tap_labeled(more_button, &MORE_LABELS)?;
    if !wait_for_focus("ChatRoomSideActivity", Duration::from_secs(5)) {
        return Err(NoaError::AndroidUnavailable(
            "채팅방 더보기 화면을 열지 못했습니다".to_string(),
        ));
    }
    Ok(display)
}

fn current_chat_matches(expected_room_name: Option<&str>) -> bool {
    if !chat_room_is_focused() {
        return false;
    }
    dump_nodes()
        .ok()
        .and_then(|nodes| select_chat_title(&nodes))
        .is_some_and(|title| room_title_matches(&title, expected_room_name))
}

fn wait_for_ui_idle() {
    #[cfg(target_os = "android")]
    if let Err(error) = super::ui_agent::wait_for_idle() {
        tracing::warn!(%error, "UiAutomator 화면 전환 대기 실패");
    }
}

pub fn resend(room_id: i64, profile: i32, message: &str, attachment: &str) -> Result<(), NoaError> {
    let profile = profile.to_string();
    run(
        "/system/bin/am",
        &["force-stop", "--user", &profile, KAKAO_PACKAGE],
    )?;
    let room = room_id.to_string();
    run(
        "/system/bin/am",
        &[
            "start",
            "--user",
            &profile,
            "-W",
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
    let indicator = wait_for(Duration::from_secs(15), |nodes| {
        select_indicator(nodes, message, attachment)
    })
    .ok_or_else(|| {
        NoaError::AndroidUnavailable("현재 채팅 화면에서 재전송 표시를 찾지 못했습니다".to_string())
    })?;
    tap(indicator)?;
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
    tap(confirmation)
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
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(nodes) = dump_nodes() {
            if chat_room_is_focused()
                && let Some(title) = select_chat_title(&nodes)
            {
                return Ok(OpenChatDestination::Entered(title));
            }
            if let Some((title, button)) = select_open_chat_cover(&nodes) {
                return Ok(OpenChatDestination::Cover(title, button));
            }
        }
        if !wait_for_next_probe(deadline) {
            return Err(NoaError::AndroidUnavailable(
                "오픈채팅 입장 화면을 찾지 못했습니다".to_string(),
            ));
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
            "-W",
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

fn room_title_matches(actual: &str, expected: Option<&str>) -> bool {
    expected.is_none_or(|expected| actual.trim() == expected.trim())
}

fn select_open_chat_cover(nodes: &[UiNode]) -> Option<(String, (i32, i32, i32, i32))> {
    let title = nodes
        .iter()
        .find(|node| node.resource_id == "com.kakao.talk.openlink:id/title")?
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
        return nodes
            .iter()
            .find(|node| {
                node.text.trim() == profile
                    || node
                        .description
                        .strip_prefix(profile)
                        .is_some_and(|value| value.starts_with(','))
            })
            .and_then(|node| node.bounds.map(|bounds| (profile.to_string(), bounds)));
    }
    nodes
        .iter()
        .find(|node| {
            node.resource_id == "com.kakao.talk.openlink:id/profile_name"
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
        .any(|node| node.resource_id == "com.kakao.talk.openlink:id/profile_name");
    !cover_is_visible && profile_is_visible
}

fn chat_room_is_focused() -> bool {
    focused_app().is_some_and(|value| value.contains("ChatRoomHolderActivity"))
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
    nodes
        .iter()
        .filter(|node| node.resource_id == "com.kakao.talk:id/resend_indicator")
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
            MORE_LABELS,
            SETTINGS_LABELS,
            LEAVE_CHATROOM_LABELS,
            LEAVE_LABELS,
            KICK_MEMBER_LABELS,
            REMOVE_LABELS,
            RESEND_LABELS,
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
    }

    #[test]
    fn rejects_a_stale_chat_room_title() {
        assert!(room_title_matches("Jordy Test2", Some("Jordy Test2")));
        assert!(!room_title_matches(
            "모비올 운영사-본사 소통방",
            Some("Jordy Test2")
        ));
        assert!(room_title_matches("알 수 없는 방", None));
    }

    #[test]
    fn selects_requested_or_first_open_profile() {
        let xml = r#"<hierarchy><node text="" resource-id="" class="android.widget.Button" content-desc="지민, Set as My Profile" clickable="true" scrollable="false" bounds="[0,100][720,200]"/><node text="지민" resource-id="com.kakao.talk.openlink:id/profile_name" class="android.widget.TextView" content-desc="" clickable="false" scrollable="false" bounds="[120,120][200,180]"/><node text="Kakao Friends" resource-id="com.kakao.talk.openlink:id/profile_name" class="android.widget.TextView" content-desc="" clickable="false" scrollable="false" bounds="[120,220][300,280]"/></hierarchy>"#;
        let nodes = parse_nodes(xml);
        assert_eq!(
            select_profile(&nodes, Some("지민")),
            Some(("지민".to_string(), (0, 100, 720, 200)))
        );
        assert_eq!(
            select_profile(&nodes, None),
            Some(("지민".to_string(), (120, 120, 200, 180)))
        );
        assert!(select_profile(&nodes, Some("없는 프로필")).is_none());
        assert!(is_profile_picker(&nodes));
    }

    #[test]
    fn selects_open_chat_cover_and_entered_room() {
        let cover = parse_nodes(
            r#"<hierarchy><node text="Jordy Test2" resource-id="com.kakao.talk.openlink:id/title" class="android.widget.TextView" content-desc="" clickable="false" scrollable="false" bounds="[40,578][566,643]"/><node text="" resource-id="com.kakao.talk.openlink:id/join_layout" class="android.widget.Button" content-desc="Join Open Chat" clickable="true" scrollable="false" bounds="[0,1088][720,1184]"/></hierarchy>"#,
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
    }
}
