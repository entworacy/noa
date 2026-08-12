use std::{env, path::PathBuf};

use serde::Serialize;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct IrisHookConfig {
    pub enabled: bool,
    pub bridge_url: String,
    pub config_path: PathBuf,
    pub token: String,
    pub types: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IrisHookFile<'a> {
    bridge_url: &'a str,
    token: &'a str,
    types: &'a [String],
}

impl IrisHookConfig {
    pub async fn publish(&self) -> std::io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if let Some(parent) = self.config_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let payload = serde_json::to_vec_pretty(&IrisHookFile {
            bridge_url: &self.bridge_url,
            token: &self.token,
            types: &self.types,
        })?;
        let temporary = self.config_path.with_extension("json.tmp");
        tokio::fs::write(&temporary, payload).await?;
        set_private_permissions(&temporary).await?;
        tokio::fs::rename(temporary, &self.config_path).await
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub struct Settings {
    pub bind: String,
    pub kakao_path: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub upload_dir: PathBuf,
    pub api_token: Option<String>,
    pub max_upload_bytes: usize,
    pub poll_interval_ms: u64,
    pub snapshot_interval_ms: u64,
    pub send_interval_ms: u64,
    pub android_user_id: i32,
    pub calling_package: String,
    pub file_provider_authority: Option<String>,
    pub image_max_dimension: u32,
    pub jpeg_quality: u8,
    pub kakao_hook_enabled: bool,
    pub chatonroom_interval_ms: u64,
    pub loco_history_limit: usize,
    pub iris_hook: IrisHookConfig,
}

impl Settings {
    pub fn from_env() -> Self {
        let android_user_id = parse_env("NOA_ANDROID_USER", 0_i32);
        let is_android = cfg!(target_os = "android");
        let data_dir = env_path("NOA_DATA_DIR").unwrap_or_else(|| {
            if is_android {
                PathBuf::from("/data/local/tmp/noa")
            } else {
                PathBuf::from("./data")
            }
        });
        let upload_dir = env_path("NOA_UPLOAD_DIR").unwrap_or_else(|| {
            if is_android {
                PathBuf::from("/sdcard/Android/data/com.kakao.talk/files/noa/uploads")
            } else {
                data_dir.join("uploads")
            }
        });
        let iris_hook_enabled = parse_bool_env("NOA_IRIS_HOOK", false);
        let bind = env::var("NOA_BIND").unwrap_or_else(|_| "0.0.0.0:4000".to_string());
        let bridge_port = bind.rsplit(':').next().unwrap_or("4000");
        let iris_hook = IrisHookConfig {
            enabled: iris_hook_enabled,
            bridge_url: non_empty_env("NOA_IRIS_BRIDGE_URL")
                .unwrap_or_else(|| format!("http://127.0.0.1:{bridge_port}/internal/iris/reply")),
            config_path: env_path("NOA_IRIS_HOOK_CONFIG")
                .unwrap_or_else(|| data_dir.join("iris-hook.json")),
            token: non_empty_env("NOA_IRIS_HOOK_TOKEN")
                .unwrap_or_else(|| Uuid::new_v4().simple().to_string()),
            types: hook_types(),
        };

        Self {
            bind,
            kakao_path: env_path("NOA_KAKAO_PATH").or_else(|| find_kakao_path(android_user_id)),
            data_dir,
            upload_dir,
            api_token: non_empty_env("NOA_API_TOKEN"),
            max_upload_bytes: parse_env("NOA_MAX_UPLOAD_BYTES", 64 * 1024 * 1024),
            poll_interval_ms: parse_env("NOA_POLL_INTERVAL_MS", 250_u64).max(50),
            snapshot_interval_ms: parse_env("NOA_SNAPSHOT_INTERVAL_MS", 3_000_u64).max(500),
            send_interval_ms: parse_env("NOA_SEND_INTERVAL_MS", 300_u64),
            android_user_id,
            calling_package: non_empty_env("NOA_CALLING_PACKAGE")
                .unwrap_or_else(|| "com.android.shell".to_string()),
            file_provider_authority: non_empty_env("NOA_FILE_PROVIDER_AUTHORITY"),
            image_max_dimension: parse_env("NOA_IMAGE_MAX_DIMENSION", 4096_u32).max(256),
            jpeg_quality: parse_env("NOA_JPEG_QUALITY", 85_u8).clamp(50, 95),
            kakao_hook_enabled: parse_bool_env("KAKAO_HOOK_ENABLED", true),
            chatonroom_interval_ms: parse_env("NOA_CHATONROOM_INTERVAL_MS", 10_000_u64),
            loco_history_limit: parse_env("NOA_LOCO_HISTORY_LIMIT", 1_000_usize).clamp(100, 10_000),
            iris_hook,
        }
    }

    pub fn audit_db_path(&self) -> PathBuf {
        self.data_dir.join("noa.db")
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_path(name: &str) -> Option<PathBuf> {
    non_empty_env(name).map(PathBuf::from)
}

fn parse_env<T>(name: &str, default: T) -> T
where
    T: std::str::FromStr,
{
    non_empty_env(name)
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn parse_bool_env(name: &str, default: bool) -> bool {
    non_empty_env(name)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn hook_types() -> Vec<String> {
    let requested =
        non_empty_env("NOA_IRIS_HOOK_TYPES").unwrap_or_else(|| "file,markdown,custom".to_string());
    let types: Vec<String> = requested
        .split(',')
        .map(str::trim)
        .filter(|value| matches!(*value, "file" | "markdown" | "custom"))
        .map(str::to_string)
        .collect();
    if types.is_empty() {
        vec![
            "file".to_string(),
            "markdown".to_string(),
            "custom".to_string(),
        ]
    } else {
        types
    }
}

#[cfg(unix)]
async fn set_private_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await
}

#[cfg(not(unix))]
async fn set_private_permissions(_: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

fn find_kakao_path(user_id: i32) -> Option<PathBuf> {
    [
        PathBuf::from(format!("/data/user/{user_id}/com.kakao.talk")),
        PathBuf::from("/data/data/com.kakao.talk"),
        PathBuf::from(format!(
            "/data_mirror/data_ce/null/{user_id}/com.kakao.talk"
        )),
        PathBuf::from("/data_mirror/data_ce/null/0/com.kakao.talk"),
    ]
    .into_iter()
    .find(|path| path.join("databases/KakaoTalk.db").exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn publishes_private_iris_hook_configuration() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("iris-hook.json");
        let hook = IrisHookConfig {
            enabled: true,
            bridge_url: "http://127.0.0.1:4000/internal/iris/reply".to_string(),
            config_path: path.clone(),
            token: "secret".to_string(),
            types: vec![
                "file".to_string(),
                "markdown".to_string(),
                "custom".to_string(),
            ],
        };
        hook.publish().await.unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        assert_eq!(json["token"], "secret");
        assert_eq!(json["types"][0], "file");
        assert_eq!(json["types"][1], "markdown");
        assert_eq!(json["types"][2], "custom");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
