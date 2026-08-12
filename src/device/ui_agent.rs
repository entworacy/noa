use std::{
    fs,
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

const AGENT_JAR: &[u8] = include_bytes!("../../assets/noa-uiautomator.jar");
const AGENT_DIR: &str = "/data/local/tmp/noa";
const AGENT_PATH: &str = "/data/local/tmp/noa/noa-uiautomator.jar";
const AGENT_STAGING_PATH: &str = "/data/local/tmp/noa/noa-uiautomator.jar.install";
const AGENT_LOG_PATH: &str = "/data/local/tmp/noa-uiautomator.log";
const AGENT_CLASS: &str = "dev.noa.UiAgent";
const AGENT_VERSION: &str = "NOA_UI_7";
const AGENT_PORT: u16 = 47123;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const START_TIMEOUT: Duration = Duration::from_secs(5);
const START_POLL_INTERVAL: Duration = Duration::from_millis(25);
static START_LOCK: Mutex<()> = Mutex::new(());

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

pub fn wait_for_idle() -> Result<(), String> {
    if let Err(error) = ensure_agent() {
        stop_agent();
        return Err(error);
    }
    match request("WAIT_IDLE") {
        Ok(response) if response == "OK" => Ok(()),
        Ok(response) => Err(format!("UiAutomator 에이전트 대기 실패: {response}")),
        Err(error) => Err(error),
    }
}

fn ensure_agent() -> Result<(), String> {
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
            return Ok(());
        }
        if !wait_for_next_probe(deadline) {
            return Err("UiAutomator 에이전트 시작 시간 초과".to_string());
        }
    }
}

fn publish_agent() -> Result<(), String> {
    fs::create_dir_all(AGENT_DIR)
        .map_err(|error| format!("UiAutomator 에이전트 디렉터리 생성 실패: {error}"))?;
    if fs::read(AGENT_PATH).is_ok_and(|current| current == AGENT_JAR) {
        return Ok(());
    }
    fs::write(AGENT_STAGING_PATH, AGENT_JAR)
        .map_err(|error| format!("UiAutomator 에이전트 기록 실패: {error}"))?;
    fs::rename(AGENT_STAGING_PATH, AGENT_PATH)
        .map_err(|error| format!("UiAutomator 에이전트 교체 실패: {error}"))
}

fn launch_agent() -> Result<(), String> {
    let stdout = fs::File::create(AGENT_LOG_PATH)
        .map_err(|error| format!("UiAutomator 에이전트 로그 생성 실패: {error}"))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("UiAutomator 에이전트 로그 복제 실패: {error}"))?;
    Command::new("/system/bin/uiautomator")
        .args(["runtest", AGENT_PATH, "-c", AGENT_CLASS, "--nohup"])
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
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, AGENT_PORT);
    let mut stream = TcpStream::connect_timeout(&address.into(), CONNECT_TIMEOUT)
        .map_err(|error| format!("UiAutomator 에이전트 연결 실패: {error}"))?;
    stream
        .set_read_timeout(Some(REQUEST_TIMEOUT))
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
