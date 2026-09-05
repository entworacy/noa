use std::{
    fs,
    time::{Duration, Instant},
};
use tracing::{info, warn};
pub(super) fn stable_process_target(
    observed: &mut Option<(u32, Instant)>,
    discovered: Option<u32>,
) -> Option<u32> {
    let Some(pid) = discovered else {
        *observed = None;
        return None;
    };
    match observed {
        Some((observed_pid, since)) if *observed_pid == pid => {
            (since.elapsed() >= Duration::from_secs(2)).then_some(pid)
        }
        _ => {
            *observed = Some((pid, Instant::now()));
            None
        }
    }
}
pub(super) fn iris_process_pid() -> Option<u32> {
    let entries = fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(command) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let mut arguments = command
            .split(|byte| *byte == 0)
            .filter(|value| !value.is_empty());
        let Some(executable) = arguments.next() else {
            continue;
        };
        if executable.ends_with(b"app_process")
            && arguments.any(|value| value == b"party.qwer.iris.Main")
        {
            return Some(pid);
        }
    }
    None
}

pub(super) fn kakao_process_pid() -> Option<u32> {
    let entries = fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(command) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if command.split(|byte| *byte == 0).next() == Some(b"com.kakao.talk") {
            return Some(pid);
        }
    }
    None
}

pub(super) fn log_kakao_process_discovery(pid: Option<u32>) {
    if let Some(pid) = pid {
        info!(pid, "KakaoTalk 메인 프로세스 탐지");
        return;
    }

    let mut candidates = Vec::new();
    let mut unreadable_cmdlines = 0_u32;
    let Ok(entries) = fs::read_dir("/proc") else {
        warn!("KakaoTalk 프로세스 탐지 진단에서 /proc를 읽지 못했습니다");
        return;
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let command = match fs::read(entry.path().join("cmdline")) {
            Ok(command) => command,
            Err(_) => {
                unreadable_cmdlines = unreadable_cmdlines.saturating_add(1);
                continue;
            }
        };
        let executable = command.split(|byte| *byte == 0).next().unwrap_or_default();
        if executable.starts_with(b"com.kakao.talk") {
            candidates.push(format!("{pid}:{}", String::from_utf8_lossy(executable)));
        }
    }
    info!(
        candidates = ?candidates,
        unreadable_cmdlines,
        "KakaoTalk 메인 프로세스를 찾지 못했습니다"
    );
}
