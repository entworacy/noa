use std::{
    process::Command,
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    thread,
    time::Duration,
};

use jni::vm::{AttachConfig, ScopeToken};
use tokio::sync::oneshot;
use url::Url;

use super::{
    envelope::{self, Outbound},
    framework::FrameworkChannel,
    vm::RuntimeVm,
};
use crate::{asset::PreparedAsset, failure::NoaError, settings::Settings};

enum Job {
    Files {
        room_id: i64,
        assets: Vec<PreparedAsset>,
    },
    Words {
        room_id: i64,
        text: String,
        thread_id: Option<i64>,
    },
    Markdown {
        room_id: i64,
        text: String,
    },
}

struct Ticket {
    job: Job,
    completion: oneshot::Sender<Result<(), String>>,
}

#[derive(Clone)]
pub struct OutboundQueue {
    input: SyncSender<Ticket>,
}

impl OutboundQueue {
    pub fn connect(config: &Settings) -> Result<Self, NoaError> {
        let vm = unsafe { RuntimeVm::launch() }.map_err(NoaError::AndroidUnavailable)?;
        let referer = notification_token(config).unwrap_or_default();
        let (input, output) = mpsc::sync_channel(64);
        let identity = config.calling_package.clone();
        let profile = config.android_user_id;
        let provider = config.file_provider_authority.clone();
        let cadence = Duration::from_millis(config.send_interval_ms);
        thread::Builder::new()
            .name("noa-device".to_string())
            .spawn(move || serve(vm, output, profile, identity, provider, referer, cadence))
            .map_err(|error| NoaError::AndroidUnavailable(error.to_string()))?;
        Ok(Self { input })
    }

    pub async fn deliver_files(
        &self,
        room_id: i64,
        assets: Vec<PreparedAsset>,
    ) -> Result<(), NoaError> {
        self.submit(Job::Files { room_id, assets }).await
    }

    pub async fn deliver_text(
        &self,
        room_id: i64,
        text: String,
        thread_id: Option<i64>,
    ) -> Result<(), NoaError> {
        self.submit(Job::Words {
            room_id,
            text,
            thread_id,
        })
        .await
    }

    pub async fn deliver_markdown(&self, room_id: i64, text: String) -> Result<(), NoaError> {
        self.submit(Job::Markdown { room_id, text }).await
    }

    async fn submit(&self, job: Job) -> Result<(), NoaError> {
        let (completion, response) = oneshot::channel();
        self.input
            .try_send(Ticket { job, completion })
            .map_err(|error| match error {
                TrySendError::Full(_) => {
                    NoaError::AndroidUnavailable("Android 전송 대기열이 가득 찼습니다".to_string())
                }
                TrySendError::Disconnected(_) => {
                    NoaError::AndroidUnavailable("Android 전송 작업자가 종료되었습니다".to_string())
                }
            })?;
        response
            .await
            .map_err(|_| {
                NoaError::AndroidUnavailable("Android 전송 결과가 끊겼습니다".to_string())
            })?
            .map_err(NoaError::AndroidUnavailable)
    }
}

fn serve(
    vm: jni::JavaVM,
    output: Receiver<Ticket>,
    profile: i32,
    identity: String,
    provider: Option<String>,
    referer: String,
    cadence: Duration,
) {
    let mut scope = ScopeToken::default();
    let Ok(mut environment) =
        (unsafe { vm.attach_current_thread_guard(AttachConfig::default, &mut scope) })
    else {
        reject_remaining(output, "ART 작업 스레드 연결 실패");
        return;
    };
    let channel = match FrameworkChannel::attach(environment.borrow_env_mut(), profile, &identity) {
        Ok(channel) => channel,
        Err(error) => {
            reject_remaining(output, &format!("Android framework 연결 실패: {error:?}"));
            return;
        }
    };
    for ticket in output {
        let result = execute(
            environment.borrow_env_mut(),
            &channel,
            provider.as_deref(),
            &referer,
            ticket.job,
        )
        .map_err(|error| format!("KakaoTalk 전송 실패: {error:?}"));
        let _ = ticket.completion.send(result);
        thread::sleep(cadence);
    }
}

fn reject_remaining(output: Receiver<Ticket>, reason: &str) {
    for ticket in output {
        let _ = ticket.completion.send(Err(reason.to_string()));
    }
}

fn execute(
    env: &mut jni::Env<'_>,
    channel: &FrameworkChannel,
    provider: Option<&str>,
    referer: &str,
    job: Job,
) -> jni::errors::Result<()> {
    let prepared = match job {
        Job::Words {
            room_id,
            text,
            thread_id,
        } => {
            if referer.is_empty() {
                return Err(jni::errors::Error::NullPtr("KakaoTalk 알림 참조값"));
            }
            envelope::assemble(
                env,
                Outbound::Words {
                    referer,
                    room_id,
                    text: &text,
                    thread_id,
                },
            )?
        }
        Job::Files { room_id, assets } => {
            let locations: Vec<String> = assets
                .iter()
                .map(|asset| asset_location(asset, provider))
                .collect();
            let mime = collective_mime(&assets);
            envelope::assemble(
                env,
                Outbound::Files {
                    room_id,
                    uris: &locations,
                    mime: &mime,
                    title: (assets.len() == 1).then(|| assets[0].file_name.as_str()),
                },
            )?
        }
        Job::Markdown { room_id, text } => envelope::assemble(
            env,
            Outbound::Markdown {
                room_id,
                text: &text,
            },
        )?,
    };
    channel.transmit(env, prepared)
}

fn collective_mime(assets: &[PreparedAsset]) -> String {
    if assets.len() == 1 {
        return assets[0].mime_type.clone();
    }
    if assets
        .iter()
        .all(|asset| asset.mime_type.starts_with("image/"))
    {
        "image/*".to_string()
    } else if assets
        .iter()
        .all(|asset| asset.mime_type.starts_with("video/"))
    {
        "video/*".to_string()
    } else {
        "*/*".to_string()
    }
}

fn asset_location(asset: &PreparedAsset, provider: Option<&str>) -> String {
    if is_kakao_external_path(&asset.path) {
        return kakao_file_provider_location(&asset.path);
    }
    if let Some(authority) = provider
        && let Ok(mut location) = Url::parse(&format!("content://{authority}"))
    {
        location.set_path(&asset.path.to_string_lossy());
        location
            .query_pairs_mut()
            .append_pair("name", &asset.file_name)
            .append_pair("mimeType", &asset.mime_type);
        return location.to_string();
    }
    if let Some(location) = media_store_location(asset) {
        return location;
    }
    Url::from_file_path(&asset.path)
        .map(String::from)
        .unwrap_or_else(|_| format!("file://{}", asset.path.to_string_lossy()))
}

fn is_kakao_external_path(path: &std::path::Path) -> bool {
    path.to_string_lossy()
        .starts_with("/sdcard/Android/data/com.kakao.talk/files/")
}

fn kakao_file_provider_location(path: &std::path::Path) -> String {
    let value = storage_path(path);
    let relative = value.strip_prefix("/storage/").unwrap_or(&value);
    let mut location =
        Url::parse("content://com.kakao.talk.FileProvider").expect("KakaoTalk FileProvider URI");
    location.set_path(&format!("/external_files/{relative}"));
    location.to_string()
}

fn media_store_location(asset: &PreparedAsset) -> Option<String> {
    let collection = if asset.mime_type.starts_with("image/") {
        "content://media/external_primary/images/media"
    } else {
        "content://media/external_primary/file"
    };
    let storage_path = storage_path(&asset.path);
    let escaped_path = storage_path.replace('\'', "''");
    let where_clause = format!("_data='{escaped_path}'");
    if let Some(id) = query_media_id(collection, &where_clause) {
        return Some(format!("{collection}/{id}"));
    }

    let data_binding = format!("_data:s:{storage_path}");
    let mime_binding = format!("mime_type:s:{}", asset.mime_type);
    let title_binding = format!("title:s:{}", asset.file_name);
    let display_binding = format!("_display_name:s:{}", asset.file_name);
    let inserted = Command::new("/system/bin/content")
        .args([
            "insert",
            "--uri",
            collection,
            "--bind",
            &data_binding,
            "--bind",
            &mime_binding,
            "--bind",
            &title_binding,
            "--bind",
            &display_binding,
        ])
        .output()
        .is_ok_and(|output| output.status.success());
    if !inserted {
        return None;
    }
    query_media_id(collection, &where_clause).map(|id| format!("{collection}/{id}"))
}

fn query_media_id(collection: &str, where_clause: &str) -> Option<String> {
    let output = Command::new("/system/bin/content")
        .args([
            "query",
            "--uri",
            collection,
            "--projection",
            "_id:_data",
            "--where",
            where_clause,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split("_id=")
        .nth(1)
        .and_then(|value| value.split([',', '\n', '\r']).next())
        .map(str::trim)
        .filter(|value| {
            !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
        })
        .map(str::to_string)
}

fn storage_path(path: &std::path::Path) -> String {
    let value = path.to_string_lossy();
    value
        .strip_prefix("/sdcard/")
        .map(|suffix| format!("/storage/emulated/0/{suffix}"))
        .unwrap_or_else(|| value.into_owned())
}

fn notification_token(config: &Settings) -> Option<String> {
    let value = std::fs::read_to_string(
        config
            .kakao_path
            .as_ref()?
            .join("shared_prefs/KakaoTalk.hw.perferences.xml"),
    )
    .ok()?;
    value
        .split_once(r#"<string name="NotificationReferer">"#)?
        .1
        .split_once("</string>")
        .map(|(token, _)| token.to_string())
}
