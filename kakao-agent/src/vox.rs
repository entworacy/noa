use jni::sys::JNIEnv;

use crate::{
    Operation, app_class, bool_value, call_static_object, call_static_void, int_value, java_string,
    long_value, new_string, object_value,
};

mod audio;

pub(crate) use audio::{install_hook, process_audio, push_audio, start_audio, stop_audio};

pub(crate) unsafe fn dispatch_main(
    env: *mut JNIEnv,
    id: u64,
    operation: Operation,
) -> Result<(), String> {
    let controller = unsafe { app_class(env, "dev.noa.kakao.VoxController")? };
    match operation {
        Operation::VoxStartCall {
            room,
            caller,
            peers,
            open_chat,
            team_chat,
            group_chat,
        } => {
            let peers = peers
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let peers = unsafe { new_string(env, &peers)? };
            unsafe {
                call_static_void(
                    env,
                    controller,
                    "startCecall",
                    "(JJJLjava/lang/String;ZZZ)V",
                    &[
                        long_value(id as i64),
                        long_value(room),
                        long_value(caller),
                        object_value(peers),
                        bool_value(open_chat),
                        bool_value(team_chat),
                        bool_value(group_chat),
                    ],
                )
            }
        }
        Operation::VoxCreateRoom { room, title } => {
            let title = unsafe { new_string(env, &title)? };
            unsafe {
                call_static_void(
                    env,
                    controller,
                    "createVoiceroom",
                    "(JJLjava/lang/String;)V",
                    &[long_value(id as i64), long_value(room), object_value(title)],
                )
            }
        }
        Operation::VoxJoinRoom {
            room,
            call,
            host_v4,
            host_v6,
            port,
        } => {
            let host_v4 = unsafe { new_string(env, &host_v4)? };
            let host_v6 = unsafe { new_string(env, &host_v6)? };
            unsafe {
                call_static_void(
                    env,
                    controller,
                    "joinVoiceroom",
                    "(JJJLjava/lang/String;Ljava/lang/String;I)V",
                    &[
                        long_value(id as i64),
                        long_value(room),
                        long_value(call),
                        object_value(host_v4),
                        object_value(host_v6),
                        int_value(port),
                    ],
                )
            }
        }
        Operation::VoxLeave { kind, room } => {
            let kind = unsafe { new_string(env, &kind)? };
            unsafe {
                call_static_void(
                    env,
                    controller,
                    "leave",
                    "(JLjava/lang/String;J)V",
                    &[long_value(id as i64), object_value(kind), long_value(room)],
                )
            }
        }
        _ => Err("VOX main-thread operation mismatch".to_string()),
    }
}

pub(crate) fn status() -> Result<String, String> {
    let mut value = crate::with_env(|env| unsafe {
        let controller = app_class(env, "dev.noa.kakao.VoxController")?;
        let text = call_static_object(env, controller, "status", "()Ljava/lang/String;", &[])?;
        java_string(env, text.cast())
    })
    .and_then(|text| {
        serde_json::from_str::<serde_json::Value>(&text)
            .map_err(|error| format!("parse VOX status: {error}"))
    })?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "audio".to_string(),
            serde_json::to_value(audio::status()).map_err(|error| error.to_string())?,
        );
    }
    serde_json::to_string(&value).map_err(|error| error.to_string())
}
