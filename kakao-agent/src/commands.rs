use std::{
    collections::{HashMap, hash_map::Entry},
    sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{
    ACTION_CHATONROOM, ACTION_HIDE_MESSAGE, ACTION_KICK, ACTION_LOAD_OPEN_CHAT_MEMBER, ACTION_SEND,
    ACTION_VOX, actions, packets, post_main, vox, with_env,
};

#[derive(Deserialize)]
pub(crate) struct Request {
    pub(crate) token: String,
    pub(crate) id: u64,
    action: String,
    room: Option<i64>,
    row: Option<i64>,
    log: Option<i64>,
    #[serde(rename = "logType")]
    log_type: Option<i32>,
    message: Option<String>,
    user: Option<i64>,
    link: Option<i64>,
    url: Option<String>,
    #[serde(rename = "profileId")]
    profile_id: Option<String>,
    #[serde(rename = "profileKind")]
    profile_kind: Option<String>,
    nickname: Option<String>,
    #[serde(rename = "profileImageUrl")]
    profile_image_url: Option<String>,
    caller: Option<i64>,
    peers: Option<Vec<i64>>,
    #[serde(rename = "openChat")]
    open_chat: Option<bool>,
    #[serde(rename = "teamChat")]
    team_chat: Option<bool>,
    #[serde(rename = "groupChat")]
    group_chat: Option<bool>,
    title: Option<String>,
    call: Option<i64>,
    #[serde(rename = "hostV4")]
    host_v4: Option<String>,
    #[serde(rename = "hostV6")]
    host_v6: Option<String>,
    port: Option<i32>,
    kind: Option<String>,
    mode: Option<String>,
    audio: Option<String>,
}

#[derive(Clone)]
pub(crate) enum Operation {
    Send {
        room: i64,
        row: i64,
    },
    Kick {
        room: i64,
        user: i64,
    },
    HideMessage {
        room: i64,
        log: i64,
        log_type: i32,
        message: String,
    },
    ChatOnRoom {
        room: i64,
    },
    LoadOpenChatMember {
        room: i64,
        user: i64,
    },
    ShareOpenProfile {
        link: i64,
    },
    JoinOpenChat {
        url: String,
        profile_id: String,
        profile_kind: String,
        nickname: String,
        profile_image_url: Option<String>,
    },
    VoxStartCall {
        room: i64,
        caller: i64,
        peers: Vec<i64>,
        open_chat: bool,
        team_chat: bool,
        group_chat: bool,
    },
    VoxCreateRoom {
        room: i64,
        title: String,
    },
    VoxJoinRoom {
        room: i64,
        call: i64,
        host_v4: String,
        host_v6: String,
        port: i32,
    },
    VoxLeave {
        kind: String,
        room: i64,
    },
    VoxStatus,
    VoxAudioStart {
        mode: String,
    },
    VoxAudioPush {
        audio: String,
    },
    VoxAudioStop,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenChatJoinResult {
    pub(crate) room_name: String,
    pub(crate) profile_applied: bool,
}

pub(crate) type CommandResult = Result<Option<String>, String>;

#[derive(Default)]
struct Progress {
    loaded: bool,
    result: Option<CommandResult>,
}

struct CommandState {
    operation: Operation,
    progress: Mutex<Progress>,
    changed: Condvar,
}

static COMMANDS: OnceLock<Mutex<HashMap<u64, Arc<CommandState>>>> = OnceLock::new();

pub(crate) fn execute(request: Request) -> (u64, CommandResult) {
    let id = request.id;
    let operation = match Operation::try_from(request) {
        Ok(operation) => operation,
        Err(error) => return (id, Err(error)),
    };
    let active = match ActiveCommand::register(id, operation) {
        Ok(active) => active,
        Err(error) => return (id, Err(error)),
    };
    let result = match &active.state.operation {
        Operation::Send { .. } => execute_send(id, &active.state),
        Operation::Kick { .. } => execute_kick(id, &active.state),
        Operation::HideMessage { .. } => execute_main(id, &active.state, ACTION_HIDE_MESSAGE),
        Operation::ChatOnRoom { .. } => execute_chat_on_room(id, &active.state),
        Operation::LoadOpenChatMember { .. } => execute_load_open_chat_member(id, &active.state),
        Operation::ShareOpenProfile { .. } => execute_share_open_profile(&active.state),
        Operation::JoinOpenChat { .. } => execute_join_open_chat(&active.state),
        Operation::VoxStartCall { .. }
        | Operation::VoxCreateRoom { .. }
        | Operation::VoxJoinRoom { .. }
        | Operation::VoxLeave { .. } => execute_vox_main(id, &active.state),
        Operation::VoxStatus => vox::status().map(Some),
        Operation::VoxAudioStart { mode } => vox::start_audio(mode).map(Some),
        Operation::VoxAudioPush { audio } => vox::push_audio(audio).map(Some),
        Operation::VoxAudioStop => vox::stop_audio().map(Some),
    };
    (id, result)
}

impl TryFrom<Request> for Operation {
    type Error = String;

    fn try_from(request: Request) -> Result<Self, Self::Error> {
        match request.action.as_str() {
            "send-custom" => match (request.room, request.row) {
                (Some(room), Some(row)) => Ok(Self::Send { room, row }),
                _ => Err("room and row are required".to_string()),
            },
            "kick-member" => match (request.room, request.user) {
                (Some(room), Some(user)) => Ok(Self::Kick { room, user }),
                _ => Err("room and user are required".to_string()),
            },
            "hide-message" => {
                match (request.room, request.log, request.log_type, request.message) {
                    (Some(room), Some(log), Some(log_type), Some(message))
                        if room > 0 && log > 0 && log_type >= 0 =>
                    {
                        Ok(Self::HideMessage {
                            room,
                            log,
                            log_type,
                            message,
                        })
                    }
                    _ => Err(
                        "positive room and log, non-negative logType, and message are required"
                            .to_string(),
                    ),
                }
            }
            "chat-on-room" => match request.room {
                Some(room) => Ok(Self::ChatOnRoom { room }),
                None => Err("room is required".to_string()),
            },
            "load-open-chat-member" => match (request.room, request.user) {
                (Some(room), Some(user)) if room > 0 && user > 0 => {
                    Ok(Self::LoadOpenChatMember { room, user })
                }
                _ => Err("positive room and user are required".to_string()),
            },
            "share-open-profile" => match request.link {
                Some(link) if link > 0 => Ok(Self::ShareOpenProfile { link }),
                _ => Err("positive link is required".to_string()),
            },
            "join-open-chat" => {
                let Some(url) = request.url else {
                    return Err("url is required".to_string());
                };
                let Some(profile_id) = request.profile_id else {
                    return Err("profileId is required".to_string());
                };
                let Some(profile_kind) = request.profile_kind else {
                    return Err("profileKind is required".to_string());
                };
                let Some(nickname) = request.nickname else {
                    return Err("nickname is required".to_string());
                };
                let profile_image_url = request.profile_image_url;
                if !actions::is_open_link_url(&url) {
                    return Err("invalid open chat URL".to_string());
                }
                if profile_id.is_empty() || profile_id.chars().count() > 128 {
                    return Err("invalid profileId".to_string());
                }
                if !matches!(profile_kind.as_str(), "kakao" | "open-profile") {
                    return Err("invalid profileKind".to_string());
                }
                if nickname.is_empty() || nickname.chars().count() > 128 {
                    return Err("invalid nickname".to_string());
                }
                if profile_image_url
                    .as_ref()
                    .is_some_and(|value| value.len() > 4096)
                {
                    return Err("profileImageUrl is too long".to_string());
                }
                Ok(Self::JoinOpenChat {
                    url,
                    profile_id,
                    profile_kind,
                    nickname,
                    profile_image_url,
                })
            }
            "vox-start-call" => {
                let room = request
                    .room
                    .filter(|value| *value > 0)
                    .ok_or_else(|| "positive room is required".to_string())?;
                let caller = request
                    .caller
                    .filter(|value| *value > 0)
                    .ok_or_else(|| "positive caller is required".to_string())?;
                let peers = request
                    .peers
                    .filter(|values| !values.is_empty() && values.iter().all(|value| *value > 0))
                    .ok_or_else(|| "positive peers are required".to_string())?;
                if peers.contains(&caller) {
                    return Err("caller must not be included in peers".to_string());
                }
                Ok(Self::VoxStartCall {
                    room,
                    caller,
                    group_chat: request.group_chat.unwrap_or(peers.len() > 1),
                    peers,
                    open_chat: request.open_chat.unwrap_or(false),
                    team_chat: request.team_chat.unwrap_or(false),
                })
            }
            "vox-create-room" => {
                let room = request
                    .room
                    .filter(|value| *value > 0)
                    .ok_or_else(|| "positive room is required".to_string())?;
                let title = request.title.unwrap_or_default();
                if title.chars().count() > 100 {
                    return Err("voice room title is too long".to_string());
                }
                Ok(Self::VoxCreateRoom { room, title })
            }
            "vox-join-room" => {
                let room = request
                    .room
                    .filter(|value| *value > 0)
                    .ok_or_else(|| "positive room is required".to_string())?;
                let call = request
                    .call
                    .filter(|value| *value > 0)
                    .ok_or_else(|| "positive call is required".to_string())?;
                let host_v4 = request.host_v4.unwrap_or_default();
                let host_v6 = request.host_v6.unwrap_or_default();
                if host_v4.is_empty() && host_v6.is_empty() {
                    return Err("a VOX host is required".to_string());
                }
                if host_v4.len() > 255 || host_v6.len() > 255 {
                    return Err("VOX host is too long".to_string());
                }
                let port = request
                    .port
                    .filter(|value| (1..=65_535).contains(value))
                    .ok_or_else(|| "valid VOX port is required".to_string())?;
                Ok(Self::VoxJoinRoom {
                    room,
                    call,
                    host_v4,
                    host_v6,
                    port,
                })
            }
            "vox-leave" => {
                let kind = request.kind.unwrap_or_default();
                if !matches!(kind.as_str(), "cecall" | "voiceroom") {
                    return Err("kind must be cecall or voiceroom".to_string());
                }
                let room = request
                    .room
                    .filter(|value| *value > 0)
                    .ok_or_else(|| "positive room is required".to_string())?;
                Ok(Self::VoxLeave { kind, room })
            }
            "vox-status" => Ok(Self::VoxStatus),
            "vox-audio-start" => {
                let mode = request.mode.unwrap_or_else(|| "replace".to_string());
                if !matches!(mode.as_str(), "replace" | "mix") {
                    return Err("VOX audio mode must be replace or mix".to_string());
                }
                Ok(Self::VoxAudioStart { mode })
            }
            "vox-audio-push" => request
                .audio
                .filter(|value| !value.is_empty())
                .map(|audio| Self::VoxAudioPush { audio })
                .ok_or_else(|| "audio is required".to_string()),
            "vox-audio-stop" => Ok(Self::VoxAudioStop),
            _ => Err("unsupported action".to_string()),
        }
    }
}

struct ActiveCommand {
    id: u64,
    state: Arc<CommandState>,
}

impl ActiveCommand {
    fn register(id: u64, operation: Operation) -> Result<Self, String> {
        let state = Arc::new(CommandState {
            operation,
            progress: Mutex::new(Progress::default()),
            changed: Condvar::new(),
        });
        match lock(commands()).entry(id) {
            Entry::Vacant(entry) => {
                entry.insert(state.clone());
                Ok(Self { id, state })
            }
            Entry::Occupied(_) => Err(format!("command ID is already active: {id}")),
        }
    }
}

impl Drop for ActiveCommand {
    fn drop(&mut self) {
        let mut commands = lock(commands());
        if commands
            .get(&self.id)
            .is_some_and(|state| Arc::ptr_eq(state, &self.state))
        {
            commands.remove(&self.id);
        }
    }
}

fn execute_send(id: u64, state: &CommandState) -> CommandResult {
    with_env(|env| unsafe { actions::start_sending_log_load(env, id) })?;
    wait_loaded(state, Duration::from_secs(10))?;
    with_env(|env| unsafe { post_main(env, id, ACTION_SEND) })?;
    wait_result(state, Duration::from_secs(10))
}

fn execute_kick(id: u64, state: &CommandState) -> CommandResult {
    let (room, user) = match state.operation {
        Operation::Kick { room, user } => (room, user),
        _ => return Err("kick command state mismatch".to_string()),
    };
    // Refresh the requested member before resolving it from KakaoTalk's
    // in-memory open-chat member repository. The DB row can exist while that
    // repository has not loaded the member yet.
    with_env(|env| unsafe { packets::send_getmem(env, id, room, user) })?;
    wait_loaded(state, Duration::from_secs(10))?;
    with_env(|env| unsafe { post_main(env, id, ACTION_KICK) })?;
    wait_result(state, Duration::from_secs(10))
}

fn execute_main(id: u64, state: &CommandState, action: i32) -> CommandResult {
    with_env(|env| unsafe { post_main(env, id, action) })?;
    wait_result(state, Duration::from_secs(10))
}

fn execute_chat_on_room(id: u64, state: &CommandState) -> CommandResult {
    with_env(|env| unsafe { post_main(env, id, ACTION_CHATONROOM) })?;
    wait_result(state, Duration::from_secs(10))
}

fn execute_load_open_chat_member(id: u64, state: &CommandState) -> CommandResult {
    with_env(|env| unsafe { post_main(env, id, ACTION_LOAD_OPEN_CHAT_MEMBER) })?;
    wait_loaded(state, Duration::from_secs(10))?;
    Ok(None)
}

fn execute_share_open_profile(state: &CommandState) -> CommandResult {
    let link = match &state.operation {
        Operation::ShareOpenProfile { link } => *link,
        _ => return Err("open profile share command state mismatch".to_string()),
    };
    let url = with_env(|env| unsafe { actions::load_open_profile_url(env, link) })?;
    Ok(Some(url))
}

fn execute_join_open_chat(state: &CommandState) -> CommandResult {
    let Operation::JoinOpenChat {
        url,
        profile_id,
        profile_kind,
        nickname,
        profile_image_url,
    } = &state.operation
    else {
        return Err("open chat join command state mismatch".to_string());
    };
    let result = with_env(|env| unsafe {
        actions::join_open_chat(
            env,
            url,
            profile_id,
            profile_kind,
            nickname,
            profile_image_url.as_deref(),
        )
    })?;
    serde_json::to_string(&result)
        .map(Some)
        .map_err(|error| format!("serialize open chat join result: {error}"))
}

fn execute_vox_main(id: u64, state: &CommandState) -> CommandResult {
    with_env(|env| unsafe { post_main(env, id, ACTION_VOX) })?;
    wait_result(state, Duration::from_secs(35))
}

fn wait_loaded(state: &CommandState, timeout: Duration) -> Result<(), String> {
    let progress = lock(&state.progress);
    let (progress, result) = state
        .changed
        .wait_timeout_while(progress, timeout, |value| {
            !value.loaded && value.result.is_none()
        })
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(result) = progress.result.clone() {
        return result.map(|_| ());
    }
    if result.timed_out() && !progress.loaded {
        return Err("sending log load timed out".to_string());
    }
    Ok(())
}

fn wait_result(state: &CommandState, timeout: Duration) -> CommandResult {
    let progress = lock(&state.progress);
    let (progress, result) = state
        .changed
        .wait_timeout_while(progress, timeout, |value| value.result.is_none())
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(value) = progress.result.clone() {
        return value;
    }
    if result.timed_out() {
        Err("KakaoTalk native call timed out".to_string())
    } else {
        Err("KakaoTalk native call ended without a result".to_string())
    }
}

pub(crate) fn mark_loaded(id: u64) {
    if let Some(state) = lock(commands()).get(&id).cloned() {
        let mut progress = lock(&state.progress);
        progress.loaded = true;
        state.changed.notify_all();
    }
}

pub(crate) fn mark_complete(id: u64, result: Result<(), String>) {
    if let Some(state) = lock(commands()).get(&id).cloned() {
        let mut progress = lock(&state.progress);
        progress.result = Some(result.map(|_| None));
        state.changed.notify_all();
    }
}

pub(crate) fn command_operation(id: u64) -> Result<Operation, String> {
    lock(commands())
        .get(&id)
        .map(|state| state.operation.clone())
        .ok_or_else(|| "command state was removed".to_string())
}

fn commands() -> &'static Mutex<HashMap<u64, Arc<CommandState>>> {
    COMMANDS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) unsafe fn dispatch_packet_action(
    env: *mut jni::sys::JNIEnv,
    id: u64,
    operation: Operation,
) -> Result<(), String> {
    match operation {
        Operation::ChatOnRoom { room } => unsafe { packets::send_chat_on_room(env, id, room) },
        Operation::LoadOpenChatMember { room, user } => unsafe {
            packets::send_getmem(env, id, room, user)
        },
        _ => Err("packet command state mismatch".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{ActiveCommand, Operation, Request};

    fn request(value: serde_json::Value) -> Request {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn parses_supported_commands() {
        let send = Operation::try_from(request(serde_json::json!({
            "token": "secret",
            "id": 1,
            "action": "send-custom",
            "room": 10,
            "row": 20
        })))
        .unwrap();
        assert!(matches!(send, Operation::Send { room: 10, row: 20 }));

        let hide = Operation::try_from(request(serde_json::json!({
            "token": "secret",
            "id": 4,
            "action": "hide-message",
            "room": 10,
            "log": 30,
            "logType": 1,
            "message": "hello"
        })))
        .unwrap();
        assert!(matches!(
            hide,
            Operation::HideMessage {
                room: 10,
                log: 30,
                log_type: 1,
                ..
            }
        ));

        let join = Operation::try_from(request(serde_json::json!({
            "token": "secret",
            "id": 2,
            "action": "join-open-chat",
            "url": "https://open.kakao.com/o/Room123",
            "profileId": "42",
            "profileKind": "open-profile",
            "nickname": "noa"
        })))
        .unwrap();
        assert!(matches!(
            join,
            Operation::JoinOpenChat {
                profile_image_url: None,
                ..
            }
        ));

        let vox = Operation::try_from(request(serde_json::json!({
            "token": "secret",
            "id": 3,
            "action": "vox-start-call",
            "room": 10,
            "caller": 20,
            "peers": [30, 40],
            "openChat": true
        })))
        .unwrap();
        assert!(matches!(
            vox,
            Operation::VoxStartCall {
                room: 10,
                caller: 20,
                group_chat: true,
                open_chat: true,
                ..
            }
        ));
    }

    #[test]
    fn rejects_invalid_command_arguments() {
        let invalid_member = Operation::try_from(request(serde_json::json!({
            "token": "secret",
            "id": 3,
            "action": "load-open-chat-member",
            "room": 0,
            "user": 1
        })));
        assert_eq!(
            invalid_member.err().as_deref(),
            Some("positive room and user are required")
        );

        let invalid_hide = Operation::try_from(request(serde_json::json!({
            "token": "secret",
            "id": 4,
            "action": "hide-message",
            "room": 10,
            "log": 0,
            "logType": 1,
            "message": "hello"
        })));
        assert!(invalid_hide.is_err());

        let invalid_url = Operation::try_from(request(serde_json::json!({
            "token": "secret",
            "id": 4,
            "action": "join-open-chat",
            "url": "https://example.com/o/Room123",
            "profileId": "42",
            "profileKind": "open-profile",
            "nickname": "noa"
        })));
        assert_eq!(invalid_url.err().as_deref(), Some("invalid open chat URL"));

        let invalid_vox_host = Operation::try_from(request(serde_json::json!({
            "token": "secret",
            "id": 5,
            "action": "vox-join-room",
            "room": 10,
            "call": 20,
            "hostV4": "",
            "hostV6": "",
            "port": 17000
        })));
        assert_eq!(
            invalid_vox_host.err().as_deref(),
            Some("a VOX host is required")
        );
    }

    #[test]
    fn active_command_ids_cannot_be_overwritten() {
        let id = u64::MAX;
        let first = ActiveCommand::register(id, Operation::ChatOnRoom { room: 1 }).unwrap();
        let duplicate = ActiveCommand::register(id, Operation::ChatOnRoom { room: 2 });
        assert_eq!(
            duplicate.err().as_deref(),
            Some("command ID is already active: 18446744073709551615")
        );

        drop(first);
        assert!(ActiveCommand::register(id, Operation::ChatOnRoom { room: 3 }).is_ok());
    }
}
