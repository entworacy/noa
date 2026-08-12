use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use image::{DynamicImage, GenericImageView, ImageFormat, codecs::jpeg::JpegEncoder};
use serde::Serialize;
use tokio::task;
use uuid::Uuid;

use crate::{failure::NoaError, settings::Settings};

const CLEANUP_AFTER: Duration = Duration::from_secs(30 * 60);
const REENCODE_AFTER_BYTES: usize = 5 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedAsset {
    #[serde(skip)]
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub path: PathBuf,
    pub file_name: String,
    pub mime_type: String,
    pub original_bytes: usize,
    pub stored_bytes: usize,
    pub optimized: bool,
}

pub async fn stage(
    config: &Settings,
    bytes: Vec<u8>,
    requested_name: Option<&str>,
    declared_mime: Option<&str>,
) -> Result<PreparedAsset, NoaError> {
    if bytes.is_empty() {
        return Err(NoaError::BadRequest(
            "빈 파일은 전송할 수 없습니다".to_string(),
        ));
    }
    if bytes.len() > config.max_upload_bytes {
        return Err(NoaError::BadRequest(format!(
            "파일 크기가 제한({} bytes)을 초과했습니다",
            config.max_upload_bytes
        )));
    }

    let original_bytes = bytes.len();
    let guessed = infer::get(&bytes);
    let mut mime_type = guessed
        .map(|kind| kind.mime_type().to_string())
        .or_else(|| normalize_declared_mime(declared_mime))
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let mut extension = guessed
        .map(|kind| kind.extension().to_string())
        .or_else(|| extension_for_mime(&mime_type))
        .unwrap_or_else(|| "bin".to_string());

    let image_max_dimension = config.image_max_dimension;
    let jpeg_quality = config.jpeg_quality;
    let input_mime = mime_type.clone();
    let (stored, optimized, output_mime, output_extension) = task::spawn_blocking(move || {
        optimize_image(bytes, &input_mime, image_max_dimension, jpeg_quality)
    })
    .await
    .map_err(|error| NoaError::Internal(format!("이미지 처리 작업 실패: {error}")))?;

    if let Some(value) = output_mime {
        mime_type = value;
    }
    if let Some(value) = output_extension {
        extension = value;
    }
    let file_name = normalized_file_name(requested_name, &mime_type, &extension);
    let job_directory = config.upload_dir.join(Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&job_directory)
        .await
        .map_err(|error| {
            NoaError::Internal(format!("임시 업로드 디렉터리를 만들지 못했습니다: {error}"))
        })?;
    set_directory_readable(&job_directory).await?;
    let path = job_directory.join(&file_name);
    tokio::fs::write(&path, &stored).await.map_err(|error| {
        NoaError::Internal(format!("임시 업로드 파일을 저장하지 못했습니다: {error}"))
    })?;
    set_world_readable(&path).await?;

    Ok(PreparedAsset {
        path,
        file_name,
        mime_type,
        original_bytes,
        stored_bytes: stored.len(),
        optimized,
    })
}

fn optimize_image(
    bytes: Vec<u8>,
    mime_type: &str,
    max_dimension: u32,
    jpeg_quality: u8,
) -> (Vec<u8>, bool, Option<String>, Option<String>) {
    if !matches!(mime_type, "image/jpeg" | "image/png" | "image/webp") {
        return (bytes, false, None, None);
    }
    let Ok(image) = image::load_from_memory(&bytes) else {
        return (bytes, false, None, None);
    };
    let (width, height) = image.dimensions();
    if width <= max_dimension && height <= max_dimension && bytes.len() <= REENCODE_AFTER_BYTES {
        return (bytes, false, None, None);
    }

    let resized = if width > max_dimension || height > max_dimension {
        image.thumbnail(max_dimension, max_dimension)
    } else {
        image
    };
    let encoded = if resized.color().has_alpha() {
        encode_png(&resized).map(|output| (output, "image/png".to_string(), "png".to_string()))
    } else {
        encode_jpeg(&resized, jpeg_quality)
            .map(|output| (output, "image/jpeg".to_string(), "jpg".to_string()))
    };

    match encoded {
        Ok((output, output_mime, extension))
            if output.len() < bytes.len() || width > max_dimension || height > max_dimension =>
        {
            (output, true, Some(output_mime), Some(extension))
        }
        _ => (bytes, false, None, None),
    }
}

fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, image::ImageError> {
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, ImageFormat::Png)?;
    Ok(output.into_inner())
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, image::ImageError> {
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, quality).encode_image(image)?;
    Ok(output)
}

fn normalize_declared_mime(value: Option<&str>) -> Option<String> {
    let value = value?.split(';').next()?.trim().to_ascii_lowercase();
    value
        .parse::<mime::Mime>()
        .ok()
        .filter(|mime| *mime != mime::APPLICATION_OCTET_STREAM)
        .map(|mime| mime.to_string())
}

fn extension_for_mime(mime_type: &str) -> Option<String> {
    mime_guess::get_mime_extensions_str(mime_type)
        .and_then(|values| values.first())
        .map(|value| (*value).to_string())
}

fn normalized_file_name(requested: Option<&str>, mime_type: &str, extension: &str) -> String {
    let raw = requested
        .and_then(|value| value.rsplit(['/', '\\']).next())
        .unwrap_or("file");
    let mut safe: String = raw
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .take(120)
        .collect();
    if safe.trim_matches(['.', ' ']).is_empty() {
        safe = "file".to_string();
    }

    let existing_matches = Path::new(&safe)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| {
            mime_guess::from_ext(value)
                .iter()
                .any(|mime| mime.essence_str() == mime_type)
        })
        .unwrap_or(false);
    if existing_matches {
        return safe;
    }
    let stem = Path::new(&safe)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("file");
    format!("{stem}.{extension}")
}

#[cfg(unix)]
async fn set_world_readable(path: &Path) -> Result<(), NoaError> {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
        .await
        .map_err(NoaError::from)
}

#[cfg(unix)]
async fn set_directory_readable(path: &Path) -> Result<(), NoaError> {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .await
        .map_err(NoaError::from)
}

#[cfg(not(unix))]
async fn set_world_readable(_: &Path) -> Result<(), NoaError> {
    Ok(())
}

#[cfg(not(unix))]
async fn set_directory_readable(_: &Path) -> Result<(), NoaError> {
    Ok(())
}

pub fn schedule_reaping(directory: PathBuf) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            interval.tick().await;
            reap_expired(&directory).await;
        }
    });
}

async fn reap_expired(directory: &Path) {
    let cutoff = SystemTime::now()
        .checked_sub(CLEANUP_AFTER)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let Ok(mut entries) = tokio::fs::read_dir(directory).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if metadata.modified().is_ok_and(|modified| modified < cutoff) {
            if metadata.is_dir() {
                let _ = tokio::fs::remove_dir_all(entry.path()).await;
            } else if metadata.is_file() {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_is_sanitized_and_extension_is_corrected() {
        assert_eq!(
            normalized_file_name(Some("../../사진.exe"), "image/jpeg", "jpg"),
            "사진.jpg"
        );
        assert_eq!(
            normalized_file_name(Some("report.pdf"), "application/pdf", "pdf"),
            "report.pdf"
        );
    }

    #[test]
    fn small_png_is_preserved() {
        let image = DynamicImage::new_rgba8(2, 2);
        let bytes = encode_png(&image).unwrap();
        let original = bytes.clone();
        let (output, changed, mime, _) = optimize_image(bytes, "image/png", 4096, 85);
        assert!(!changed);
        assert_eq!(output, original);
        assert!(mime.is_none());
    }

    #[tokio::test]
    async fn non_image_file_is_staged_without_conversion() {
        let directory = tempfile::tempdir().unwrap();
        let config = Settings {
            bind: "127.0.0.1:0".to_string(),
            kakao_path: None,
            data_dir: directory.path().join("data"),
            upload_dir: directory.path().join("uploads"),
            api_token: None,
            max_upload_bytes: 1024,
            poll_interval_ms: 100,
            snapshot_interval_ms: 500,
            send_interval_ms: 100,
            android_user_id: 0,
            calling_package: "com.android.shell".to_string(),
            file_provider_authority: None,
            image_max_dimension: 4096,
            jpeg_quality: 85,
            kakao_hook_enabled: true,
            chatonroom_interval_ms: 10_000,
            loco_history_limit: 1_000,
            iris_hook: crate::settings::IrisHookConfig {
                enabled: false,
                bridge_url: "http://127.0.0.1:4000/internal/iris/reply".to_string(),
                config_path: directory.path().join("iris-hook.json"),
                token: "test".to_string(),
                types: vec!["image".to_string()],
            },
        };
        let original = b"%PDF-1.4\n1 0 obj\n<<>>\nendobj\n%%EOF".to_vec();
        let staged = stage(
            &config,
            original.clone(),
            Some("report"),
            Some("application/pdf"),
        )
        .await
        .unwrap();
        assert_eq!(staged.file_name, "report.pdf");
        assert_eq!(staged.mime_type, "application/pdf");
        assert!(!staged.optimized);
        assert_eq!(tokio::fs::read(staged.path).await.unwrap(), original);
    }
}
