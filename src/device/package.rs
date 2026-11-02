use crate::failure::NoaError;

/// Query PackageManager without requiring the injected KakaoTalk agent.
/// Do not cache: the installed app can change while Noa is running.
#[cfg(target_os = "android")]
pub async fn kakao_version() -> Result<String, NoaError> {
    tokio::task::spawn_blocking(|| {
        let output = std::process::Command::new("/system/bin/dumpsys")
            .args(["-t", "5", "package", "com.kakao.talk"])
            .output()
            .map_err(|error| {
                NoaError::AndroidUnavailable(format!("KakaoTalk 버전 조회 실패: {error}"))
            })?;
        if !output.status.success() {
            return Err(NoaError::AndroidUnavailable(
                "KakaoTalk 버전 조회 명령이 실패했습니다".to_string(),
            ));
        }
        version_from_dump(&String::from_utf8_lossy(&output.stdout))
    })
    .await
    .map_err(|error| NoaError::Internal(error.to_string()))?
}

#[cfg(not(target_os = "android"))]
pub async fn kakao_version() -> Result<String, NoaError> {
    Err(NoaError::AndroidUnavailable(
        "KakaoTalk 버전 조회는 Android에서만 사용할 수 있습니다".to_string(),
    ))
}

#[cfg(any(target_os = "android", test))]
fn version_from_dump(dump: &str) -> Result<String, NoaError> {
    let mut version = None;
    for line in dump.lines() {
        let Some(value) = line.trim().strip_prefix("versionName=") else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() || version.is_some_and(|previous| previous != value) {
            return Err(NoaError::AndroidUnavailable(
                "KakaoTalk 버전 정보가 비어 있거나 일치하지 않습니다".to_string(),
            ));
        }
        version = Some(value);
    }
    version.map(str::to_string).ok_or_else(|| {
        NoaError::AndroidUnavailable("KakaoTalk versionName을 찾을 수 없습니다".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_package_version_without_a_hook() {
        assert_eq!(
            version_from_dump("Packages:\n  Package [com.kakao.talk]:\n    versionCode=29260630\n    versionName=26.6.3\n    User 0: installed=true\n").unwrap(),
            "26.6.3"
        );
        assert_eq!(
            version_from_dump("versionName=26.9.1\n versionName=26.9.1").unwrap(),
            "26.9.1"
        );
    }

    #[test]
    fn rejects_missing_empty_or_conflicting_versions() {
        for dump in [
            "",
            "Can't find service: package",
            "DUMP TIMEOUT",
            "versionName=",
            "versionName=26.9.1\nversionName=26.6.3",
        ] {
            assert!(version_from_dump(dump).is_err(), "{dump}");
        }
    }
}
