use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};

const AGENT_JAR: &[u8] = include_bytes!("../../assets/noa-uiautomator.jar");
const AGENT_PATH: &str = "/data/local/tmp/noa-uiautomator.jar";
const AGENT_STAGING_PATH: &str = "/data/local/tmp/noa-uiautomator.jar.install";
const AGENT_LOG_PATH: &str = "/data/local/tmp/noa-uiautomator.log";
const AGENT_CLASS: &str = "dev.noa.UiAgent";
const AGENT_VERSION: &str = "NOA_UI_32";
const AGENT_PORT: u16 = 47123;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const START_TIMEOUT: Duration = Duration::from_secs(5);
const START_POLL_INTERVAL: Duration = Duration::from_millis(25);
static START_LOCK: Mutex<()> = Mutex::new(());

pub enum TextClickResult {
    Clicked,
    NotFound,
    Ambiguous,
}

pub enum ResendTargetClickResult {
    Clicked,
    NotFound,
    Ambiguous,
}

pub enum OpenChatDestination {
    Entered(String),
    Cover(String),
    Rejected(String),
}

pub enum OpenChatProfileDestination {
    Entered(String),
    Selected(String),
    Rejected(String),
    Ambiguous,
}

pub enum MemberProfileShareState {
    Shareable,
    NotShareable,
    Unknown,
}

pub fn dump_hierarchy() -> Result<(), String> {
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request("DUMP") {
        Ok(response) if response == "OK" => Ok(()),
        Ok(response) => {
            stop_agent();
            Err(format!("UiAutomator 에이전트 덤프 실패: {response}"))
        }
        Err(error) => {
            stop_agent();
            Err(error)
        }
    }
}

pub fn click(x: i32, y: i32) -> Result<(), String> {
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request(&format!("CLICK {x} {y}")) {
        Ok(response) if response == "OK" => Ok(()),
        Ok(response) => Err(format!("UiAutomator 에이전트 클릭 실패: {response}")),
        Err(error) => Err(error),
    }
}

pub fn click_label(label: &str) -> Result<(), String> {
    if label.contains(['\n', '\r']) {
        return Err("UiAutomator 라벨에 줄바꿈을 사용할 수 없습니다".to_string());
    }
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request(&format!("CLICK_LABEL {label}")) {
        Ok(response) if response == "OK" => Ok(()),
        Ok(response) => Err(format!("UiAutomator 에이전트 라벨 클릭 실패: {response}")),
        Err(error) => Err(error),
    }
}

pub fn click_text_at(text: &str, bounds: (i32, i32, i32, i32)) -> Result<(), String> {
    if text.contains(['\n', '\r']) {
        return Err("UiAutomator 텍스트에 줄바꿈을 사용할 수 없습니다".to_string());
    }
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    let command = format!(
        "CLICK_TEXT_AT {} {} {} {} {text}",
        bounds.0, bounds.1, bounds.2, bounds.3
    );
    match request(&command) {
        Ok(response) if response == "OK" => Ok(()),
        Ok(response) => Err(format!("UiAutomator 에이전트 텍스트 클릭 실패: {response}")),
        Err(error) => Err(error),
    }
}

pub fn wait_for_text(text: &str, timeout: Duration) -> Result<bool, String> {
    validate_text(text)?;
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    let timeout_ms = timeout.as_millis().min(10_000);
    match request_with_timeout(
        &format!("WAIT_TEXT {timeout_ms} {text}"),
        timeout + Duration::from_secs(1),
    ) {
        Ok(response) if response == "OK" => Ok(true),
        Ok(response) if response == "NOT_FOUND" => Ok(false),
        Ok(response) => Err(format!("UiAutomator 에이전트 텍스트 대기 실패: {response}")),
        Err(error) => Err(error),
    }
}

pub fn wait_for_resource_text(
    resource_id: &str,
    text: &str,
    timeout: Duration,
) -> Result<bool, String> {
    validate_resource_id(resource_id)?;
    validate_text(text)?;
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    let timeout_ms = timeout.as_millis().min(20_000);
    match request_with_timeout(
        &format!("WAIT_RESOURCE_TEXT {timeout_ms} {resource_id} {text}"),
        timeout + Duration::from_secs(1),
    ) {
        Ok(response) if response == "OK" => Ok(true),
        Ok(response) if response == "NOT_FOUND" => Ok(false),
        Ok(response) => Err(format!(
            "UiAutomator 에이전트 리소스 텍스트 대기 실패: {response}"
        )),
        Err(error) => Err(error),
    }
}

pub fn click_resource_at(resource_id: &str, bounds: (i32, i32, i32, i32)) -> Result<bool, String> {
    validate_resource_id(resource_id)?;
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    let command = format!(
        "CLICK_RESOURCE_AT {} {} {} {} {resource_id}",
        bounds.0, bounds.1, bounds.2, bounds.3
    );
    match request(&command) {
        Ok(response) if response == "OK" => Ok(true),
        Ok(response) if response == "NOT_FOUND" => Ok(false),
        Ok(response) => Err(format!("UiAutomator 에이전트 리소스 클릭 실패: {response}")),
        Err(error) => Err(error),
    }
}

pub fn click_resource(resource_id: &str) -> Result<bool, String> {
    validate_resource_id(resource_id)?;
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request(&format!("CLICK_RESOURCE {resource_id}")) {
        Ok(response) if response == "OK" => Ok(true),
        Ok(response) if response == "NOT_FOUND" => Ok(false),
        Ok(response) => Err(format!("UiAutomator 에이전트 리소스 클릭 실패: {response}")),
        Err(error) => Err(error),
    }
}

pub fn click_open_link_copy() -> Result<bool, String> {
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request_with_timeout("CLICK_OPEN_LINK_COPY", Duration::from_secs(10)) {
        Ok(response) if response == "OK" => Ok(true),
        Ok(response) if response == "NOT_FOUND" => Ok(false),
        Ok(response) => Err(format!(
            "UiAutomator 에이전트 오픈프로필 링크 복사 실패: {response}"
        )),
        Err(error) => Err(error),
    }
}

pub fn wait_open_profile_more() -> Result<bool, String> {
    click_wait_command(
        "WAIT_OPEN_PROFILE_MORE",
        "오픈프로필 공유 메뉴 대기",
        Duration::from_secs(13),
    )
}

pub fn wait_member_profile_share() -> Result<MemberProfileShareState, String> {
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request_with_timeout("WAIT_MEMBER_PROFILE_SHARE", Duration::from_secs(7)) {
        Ok(response) if response == "SHAREABLE" => Ok(MemberProfileShareState::Shareable),
        Ok(response) if response == "NOT_SHAREABLE" => Ok(MemberProfileShareState::NotShareable),
        Ok(response) if response == "NOT_FOUND" => Ok(MemberProfileShareState::Unknown),
        Ok(response) => Err(format!(
            "UiAutomator 에이전트 멤버 프로필 공유 상태 확인 실패: {response}"
        )),
        Err(error) => Err(error),
    }
}

pub fn click_open_profile_more() -> Result<bool, String> {
    click_wait_command(
        "CLICK_OPEN_PROFILE_MORE",
        "오픈프로필 공유 메뉴 선택",
        Duration::from_secs(7),
    )
}

pub fn prepare_clipboard() -> Result<(), String> {
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request("PREPARE_CLIPBOARD") {
        Ok(response) if response == "OK" => Ok(()),
        Ok(response) => Err(format!(
            "UiAutomator 에이전트 클립보드 초기화 실패: {response}"
        )),
        Err(error) => Err(error),
    }
}

pub fn wait_clipboard_change(timeout: Duration) -> Result<Option<String>, String> {
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    let timeout_ms = timeout.as_millis().min(20_000);
    let response = request_with_timeout(
        &format!("WAIT_CLIPBOARD_CHANGE {timeout_ms}"),
        timeout + Duration::from_secs(1),
    )?;
    if response == "NOT_FOUND" {
        return Ok(None);
    }
    decode_response(&response, "VALUE ")
}

pub fn click_resend_confirmation() -> Result<bool, String> {
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request_with_timeout("CLICK_RESEND_CONFIRM", Duration::from_secs(10)) {
        Ok(response) if response == "OK" => Ok(true),
        Ok(response) if response == "NOT_FOUND" => Ok(false),
        Ok(response) => Err(format!("UiAutomator 에이전트 재전송 확인 실패: {response}")),
        Err(error) => Err(error),
    }
}

pub fn click_resend_target(
    targets: &[String],
    timeout: Duration,
) -> Result<ResendTargetClickResult, String> {
    if targets.len() > 128 || targets.iter().any(|target| target.chars().count() > 100) {
        return Err("재전송 대상 문자열은 128개, 각 100자 이하여야 합니다".to_string());
    }
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    let timeout_ms = timeout.as_millis().min(20_000);
    let encoded = targets
        .iter()
        .map(|target| STANDARD_NO_PAD.encode(target.as_bytes()))
        .collect::<Vec<_>>()
        .join(" ");
    let command = if encoded.is_empty() {
        format!("CLICK_RESEND_TARGET {timeout_ms}")
    } else {
        format!("CLICK_RESEND_TARGET {timeout_ms} {encoded}")
    };
    match request_with_timeout(&command, timeout + Duration::from_secs(1)) {
        Ok(response) if response == "OK" => Ok(ResendTargetClickResult::Clicked),
        Ok(response) if response == "NOT_FOUND" => Ok(ResendTargetClickResult::NotFound),
        Ok(response) if response == "AMBIGUOUS" => Ok(ResendTargetClickResult::Ambiguous),
        Ok(response) => Err(format!(
            "UiAutomator 에이전트 재전송 대상 선택 실패: {response}"
        )),
        Err(error) => Err(error),
    }
}

pub fn scroll_click_text(text: &str) -> Result<TextClickResult, String> {
    validate_text(text)?;
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request_with_timeout(
        &format!("SCROLL_CLICK_TEXT {text}"),
        Duration::from_secs(55),
    ) {
        Ok(response) if response == "OK" => Ok(TextClickResult::Clicked),
        Ok(response) if response == "NOT_FOUND" => Ok(TextClickResult::NotFound),
        Ok(response) if response == "AMBIGUOUS" => Ok(TextClickResult::Ambiguous),
        Ok(response) => Err(format!("UiAutomator 에이전트 스크롤 선택 실패: {response}")),
        Err(error) => Err(error),
    }
}

pub fn expand_member_list() -> Result<bool, String> {
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request_with_timeout("EXPAND_MEMBER_LIST", Duration::from_secs(20)) {
        Ok(response) if response == "OK" => Ok(true),
        Ok(response) if response == "NOT_FOUND" => Ok(false),
        Ok(response) => Err(format!(
            "UiAutomator 에이전트 멤버 목록 펼치기 실패: {response}"
        )),
        Err(error) => Err(error),
    }
}

pub fn click_kick_profile(nickname: &str) -> Result<bool, String> {
    validate_text(nickname)?;
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request_with_timeout(
        &format!("CLICK_KICK_PROFILE {nickname}"),
        Duration::from_secs(10),
    ) {
        Ok(response) if response == "OK" => Ok(true),
        Ok(response) if response == "NOT_FOUND" => Ok(false),
        Ok(response) => Err(format!(
            "UiAutomator 에이전트 프로필 강퇴 선택 실패: {response}"
        )),
        Err(error) => Err(error),
    }
}

pub fn wait_open_chat_destination(
    timeout: Duration,
) -> Result<Option<OpenChatDestination>, String> {
    let response = timed_request("WAIT_OPEN_CHAT_DESTINATION", timeout)?;
    parse_open_chat_destination(&response, true)
}

pub fn wait_open_chat_profile(
    profile: &str,
    timeout: Duration,
) -> Result<Option<OpenChatProfileDestination>, String> {
    validate_text(profile)?;
    let encoded = STANDARD_NO_PAD.encode(profile.as_bytes());
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    let timeout_ms = timeout.as_millis().min(20_000);
    let response = request_with_timeout(
        &format!("WAIT_OPEN_CHAT_PROFILE {timeout_ms} {encoded}"),
        timeout + Duration::from_secs(1),
    )?;
    if response == "NOT_FOUND" {
        return Ok(None);
    }
    if response == "AMBIGUOUS" {
        return Ok(Some(OpenChatProfileDestination::Ambiguous));
    }
    if let Some(value) = decode_response(&response, "ENTERED ")? {
        return Ok(Some(OpenChatProfileDestination::Entered(value)));
    }
    if let Some(value) = decode_response(&response, "PROFILE ")? {
        return Ok(Some(OpenChatProfileDestination::Selected(value)));
    }
    if let Some(value) = decode_response(&response, "REJECTED ")? {
        return Ok(Some(OpenChatProfileDestination::Rejected(value)));
    }
    Err(format!(
        "UiAutomator 에이전트 오픈채팅 프로필 선택 실패: {response}"
    ))
}

pub fn wait_open_chat_entered(timeout: Duration) -> Result<Option<OpenChatDestination>, String> {
    let response = timed_request("WAIT_OPEN_CHAT_ENTERED", timeout)?;
    parse_open_chat_destination(&response, false)
}

pub fn click_settings() -> Result<bool, String> {
    click_wait_command("CLICK_SETTINGS", "채팅방 설정 선택", Duration::from_secs(7))
}

pub fn scroll_click_leave_chatroom() -> Result<bool, String> {
    click_wait_command(
        "SCROLL_CLICK_LEAVE_CHATROOM",
        "채팅방 나가기 선택",
        Duration::from_secs(14),
    )
}

pub fn click_leave_confirmation() -> Result<bool, String> {
    click_wait_command(
        "CLICK_LEAVE_CONFIRM",
        "채팅방 나가기 확인",
        Duration::from_secs(10),
    )
}

pub fn click_kick_confirmation() -> Result<bool, String> {
    click_wait_command("CLICK_KICK_CONFIRM", "강퇴 확인", Duration::from_secs(10))
}

pub fn wait_for_resource_gone(resource_id: &str, timeout: Duration) -> Result<bool, String> {
    validate_resource_id(resource_id)?;
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    let timeout_ms = timeout.as_millis().min(20_000);
    match request_with_timeout(
        &format!("WAIT_RESOURCE_GONE {timeout_ms} {resource_id}"),
        timeout + Duration::from_secs(1),
    ) {
        Ok(response) if response == "OK" => Ok(true),
        Ok(response) if response == "NOT_FOUND" => Ok(false),
        Ok(response) => Err(format!(
            "UiAutomator 에이전트 리소스 소멸 대기 실패: {response}"
        )),
        Err(error) => Err(error),
    }
}

fn timed_request(command: &str, timeout: Duration) -> Result<String, String> {
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    let timeout_ms = timeout.as_millis().min(20_000);
    request_with_timeout(
        &format!("{command} {timeout_ms}"),
        timeout + Duration::from_secs(1),
    )
}

fn parse_open_chat_destination(
    response: &str,
    allow_cover: bool,
) -> Result<Option<OpenChatDestination>, String> {
    if response == "NOT_FOUND" {
        return Ok(None);
    }
    if let Some(value) = decode_response(response, "ENTERED ")? {
        return Ok(Some(OpenChatDestination::Entered(value)));
    }
    if allow_cover && let Some(value) = decode_response(response, "COVER ")? {
        return Ok(Some(OpenChatDestination::Cover(value)));
    }
    if let Some(value) = decode_response(response, "REJECTED ")? {
        return Ok(Some(OpenChatDestination::Rejected(value)));
    }
    Err(format!(
        "UiAutomator 에이전트 오픈채팅 화면 대기 실패: {response}"
    ))
}

fn decode_response(response: &str, prefix: &str) -> Result<Option<String>, String> {
    let Some(encoded) = response.strip_prefix(prefix) else {
        return Ok(None);
    };
    let bytes = STANDARD_NO_PAD
        .decode(encoded)
        .or_else(|_| STANDARD.decode(encoded))
        .map_err(|error| format!("UiAutomator 에이전트 응답 디코딩 실패: {error}"))?;
    let value = String::from_utf8(bytes)
        .map_err(|error| format!("UiAutomator 에이전트 UTF-8 응답 오류: {error}"))?;
    let value = value.trim().to_string();
    if value.is_empty() {
        Err("UiAutomator 에이전트가 빈 화면 정보를 반환했습니다".to_string())
    } else {
        Ok(Some(value))
    }
}

fn click_wait_command(command: &str, operation: &str, timeout: Duration) -> Result<bool, String> {
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request_with_timeout(command, timeout) {
        Ok(response) if response == "OK" => Ok(true),
        Ok(response) if response == "NOT_FOUND" => Ok(false),
        Ok(response) => Err(format!("UiAutomator 에이전트 {operation} 실패: {response}")),
        Err(error) => Err(error),
    }
}

pub(super) fn ensure_agent() -> Result<(), String> {
    if probe() {
        return Ok(());
    }
    let _guard = START_LOCK
        .lock()
        .map_err(|_| "UiAutomator 에이전트 시작 잠금 오류".to_string())?;
    if probe() {
        return Ok(());
    }
    let previous_agent = request("PING").is_ok_and(|response| response.starts_with("NOA_UI_"));
    if previous_agent || !agent_pids().is_empty() {
        stop_agent();
    }
    publish_agent()?;
    launch_agent()?;
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if probe() {
            let status = match request("API_STATUS") {
                Ok(status) => status,
                Err(error) => {
                    stop_agent();
                    return Err(error);
                }
            };
            if !status.starts_with("OK LEGACY_UIAUTOMATOR SDK=") {
                stop_agent();
                return Err(format!(
                    "UiAutomator 에이전트 Android API 계약 검증 실패: {status}"
                ));
            }
            tracing::info!(api_status = %status, "UiAutomator 에이전트 Android API 계약 확인");
            return Ok(());
        }
        if !wait_for_next_probe(deadline) {
            let detail = fs::read_to_string(AGENT_LOG_PATH)
                .ok()
                .map(|log| {
                    let tail = log
                        .lines()
                        .rev()
                        .take(8)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join(" | ");
                    tail.chars().take(2_000).collect::<String>()
                })
                .filter(|log| !log.is_empty());
            return Err(match detail {
                Some(detail) => {
                    format!("UiAutomator 에이전트 시작 시간 초과: {detail}")
                }
                None => "UiAutomator 에이전트 시작 시간 초과".to_string(),
            });
        }
    }
}

fn publish_agent() -> Result<(), String> {
    if fs::read(AGENT_PATH).is_ok_and(|current| current == AGENT_JAR) {
        return Ok(());
    }
    fs::write(AGENT_STAGING_PATH, AGENT_JAR)
        .map_err(|error| format!("UiAutomator 에이전트 기록 실패: {error}"))?;
    fs::rename(AGENT_STAGING_PATH, AGENT_PATH)
        .map_err(|error| format!("UiAutomator 에이전트 교체 실패: {error}"))?;
    let status = Command::new("/system/bin/chmod")
        .args(["0644", AGENT_PATH])
        .status()
        .map_err(|error| format!("UiAutomator 에이전트 권한 설정 실패: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "UiAutomator 에이전트 권한 설정 실패: 종료 코드 {status}"
        ))
    }
}

fn launch_agent() -> Result<(), String> {
    let stdout = fs::File::create(AGENT_LOG_PATH)
        .map_err(|error| format!("UiAutomator 에이전트 로그 생성 실패: {error}"))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("UiAutomator 에이전트 로그 복제 실패: {error}"))?;
    Command::new("/system/xbin/su")
        .args([
            "shell",
            "/system/bin/uiautomator",
            "runtest",
            AGENT_PATH,
            "-c",
            AGENT_CLASS,
            "--nohup",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("UiAutomator 에이전트 실행 실패: {error}"))
}

fn probe() -> bool {
    request("PING").is_ok_and(|response| response == AGENT_VERSION)
}

fn request(command: &str) -> Result<String, String> {
    request_with_timeout(command, REQUEST_TIMEOUT)
}

fn request_with_timeout(command: &str, read_timeout: Duration) -> Result<String, String> {
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, AGENT_PORT);
    let mut stream = TcpStream::connect_timeout(&address.into(), CONNECT_TIMEOUT)
        .map_err(|error| format!("UiAutomator 에이전트 연결 실패: {error}"))?;
    stream
        .set_read_timeout(Some(read_timeout))
        .map_err(|error| format!("UiAutomator 에이전트 읽기 제한 설정 실패: {error}"))?;
    stream
        .set_write_timeout(Some(REQUEST_TIMEOUT))
        .map_err(|error| format!("UiAutomator 에이전트 쓰기 제한 설정 실패: {error}"))?;
    stream
        .write_all(format!("{command}\n").as_bytes())
        .map_err(|error| format!("UiAutomator 에이전트 요청 실패: {error}"))?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|error| format!("UiAutomator 에이전트 응답 실패: {error}"))?;
    let response = response.trim().to_string();
    if response.is_empty() {
        Err("UiAutomator 에이전트가 빈 응답을 반환했습니다".to_string())
    } else {
        Ok(response)
    }
}

fn validate_text(text: &str) -> Result<(), String> {
    if text.is_empty() || text.contains(['\n', '\r']) {
        Err("UiAutomator 텍스트는 비어 있거나 줄바꿈을 포함할 수 없습니다".to_string())
    } else {
        Ok(())
    }
}

fn validate_resource_id(resource_id: &str) -> Result<(), String> {
    if resource_id.is_empty() || resource_id.chars().any(char::is_whitespace) {
        Err("UiAutomator 리소스 ID는 비어 있거나 공백을 포함할 수 없습니다".to_string())
    } else {
        Ok(())
    }
}

fn stop_agent() {
    let _ = request("STOP");
    let deadline = Instant::now() + Duration::from_secs(1);
    while !agent_pids().is_empty() && wait_for_next_probe(deadline) {}
    for pid in agent_pids() {
        let _ = Command::new("/system/bin/kill").arg(pid).status();
    }
    let deadline = Instant::now() + Duration::from_millis(500);
    while !agent_pids().is_empty() && wait_for_next_probe(deadline) {}
}

fn agent_pids() -> Vec<String> {
    let Ok(output) = Command::new("/system/bin/pidof")
        .arg("uiautomator")
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter(|pid| pid.chars().all(|character| character.is_ascii_digit()))
        .map(str::to_string)
        .collect()
}

fn wait_for_next_probe(deadline: Instant) -> bool {
    let now = Instant::now();
    if now >= deadline {
        return false;
    }
    thread::park_timeout((deadline - now).min(START_POLL_INTERVAL));
    true
}
