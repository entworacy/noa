//! Android hook orchestration. Application commands and transports have separate owners.
mod commands;
mod events;
#[cfg(target_os = "android")]
mod frida;
mod iris_bridge;
mod process;
mod state;
#[cfg(target_os = "android")]
mod supervisor;

pub(super) use commands::channel_active;
pub use commands::{
    OpenChatJoinResult, chat_on_room, hide_message, join_open_chat, kick_member,
    load_open_chat_member, send_custom, share_open_profile, vox_audio_push, vox_audio_start,
    vox_audio_stop, vox_create_room, vox_join_room, vox_leave, vox_start_call, vox_status,
};
#[cfg(target_os = "android")]
pub use supervisor::launch;
