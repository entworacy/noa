use std::ptr;

use jni::sys::{JNIEnv, jobject};

use crate::{
    OpenChatJoinResult, box_long, call_static_boolean, call_static_object, find_class, find_room,
    invoke, invoke_signature_operation, new_object, new_string, object_text, object_value,
    signature_class, signature_object, unbox_long,
};

pub(crate) unsafe fn load_open_profile_url(env: *mut JNIEnv, link: i64) -> Result<String, String> {
    let resolver = unsafe { crate::app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    let url = unsafe {
        call_static_object(
            env,
            resolver,
            "openProfileUrl",
            "(J)Ljava/lang/String;",
            &[crate::long_value(link)],
        )?
    };
    let url = unsafe { object_text(env, url)? };
    if !is_open_profile_url(&url) {
        return Err(format!("invalid open profile URL for link: {link}"));
    }
    Ok(url)
}

pub(crate) fn is_open_link_url(value: &str) -> bool {
    value
        .strip_prefix("https://open.kakao.com/o/")
        .is_some_and(|token| {
            !token.is_empty()
                && token
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        })
}

fn is_open_profile_url(value: &str) -> bool {
    ["https://open.kakao.com/o/", "https://open.kakao.com/me/"]
        .into_iter()
        .find_map(|prefix| value.strip_prefix(prefix))
        .is_some_and(|token| {
            !token.is_empty()
                && token.bytes().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, b'_' | b'-')
                })
        })
}

pub(crate) unsafe fn join_open_chat(
    env: *mut JNIEnv,
    url: &str,
    profile_id: &str,
    profile_kind: &str,
    nickname: &str,
    profile_image_url: Option<&str>,
) -> Result<OpenChatJoinResult, String> {
    let connection_class = unsafe { signature_class(env, "open-link-connection")? };
    let connection_companion = unsafe { signature_object(env, "open-link-connection")? };
    let url_value = unsafe { new_string(env, url)? };
    let intent = unsafe {
        invoke_signature_operation(
            env,
            "open-link-join-intent",
            connection_companion,
            &[url_value.cast(), ptr::null_mut()],
        )?
    };
    if intent.is_null() {
        return Err("open chat join intent was empty".to_string());
    }
    let connection = unsafe {
        new_object(
            env,
            connection_class,
            "(Landroid/content/Intent;)V",
            &[object_value(intent)],
        )?
    };
    let response = unsafe {
        invoke_signature_operation(env, "open-link-connection-response", connection, &[])?
    };
    if response.is_null() {
        return Err("open chat join information was empty".to_string());
    }
    let resolver = unsafe { crate::app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    let open_link = unsafe {
        call_static_object(
            env,
            resolver,
            "convertOpenLink",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            &[object_value(response)],
        )?
    };
    if open_link.is_null() {
        return Err("open chat link conversion returned empty data".to_string());
    }
    if unsafe {
        call_static_boolean(
            env,
            resolver,
            "isOpenProfile",
            "(Ljava/lang/Object;)Z",
            &[object_value(open_link)],
        )?
    } {
        return Err("the supplied URL is an open profile, not an open chat".to_string());
    }
    let resolved_url = unsafe { invoke(env, open_link, "getUrl", &[])? };
    let resolved_url = unsafe { object_text(env, resolved_url)? };
    if resolved_url != url {
        return Err("resolved open chat URL does not match the requested URL".to_string());
    }
    let room_name = unsafe { object_text(env, invoke(env, open_link, "getName", &[])?)? };
    if room_name.trim().is_empty() {
        return Err("open chat room name was empty".to_string());
    }
    let link_id = unsafe {
        unbox_long(
            env,
            call_static_object(
                env,
                resolver,
                "openLinkId",
                "(Ljava/lang/Object;)Ljava/lang/Long;",
                &[object_value(open_link)],
            )?,
        )?
    };
    if link_id <= 0 {
        return Err("open chat link ID was invalid".to_string());
    }

    let manager = unsafe { signature_object(env, "open-link-manager")? };
    unsafe { invoke_signature_operation(env, "open-link-cache", manager, &[open_link])? };

    let repository = unsafe { signature_object(env, "room-manager")? };
    let existing = unsafe {
        call_static_object(
            env,
            resolver,
            "findOpenChatRoom",
            "(J)Ljava/lang/Object;",
            &[crate::long_value(link_id)],
        )?
    };
    if !existing.is_null() {
        if !unsafe { room_has_link(env, resolver, existing, link_id)? } {
            return Err("existing open chat link verification failed".to_string());
        }
        return Ok(OpenChatJoinResult {
            room_name,
            profile_applied: false,
        });
    }

    let profile = unsafe {
        create_open_chat_profile(env, profile_id, profile_kind, nickname, profile_image_url)?
    };
    unsafe {
        invoke_signature_operation(
            env,
            "apply-open-profile",
            manager,
            &[open_link, profile],
        )?
    };
    let pre_chat = unsafe {
        invoke_signature_operation(
            env,
            "create-open-chat",
            repository,
            &[open_link, ptr::null_mut(), ptr::null_mut(), ptr::null_mut()],
        )?
    };
    if pre_chat.is_null() {
        return Err("open chat pre-chat room was not created".to_string());
    }
    let join_manager = unsafe { signature_object(env, "room-api")? };
    let chat_id = unsafe {
        unbox_long(
            env,
            invoke_signature_operation(env, "join-link", join_manager, &[pre_chat])?,
        )?
    };
    if chat_id <= 0 {
        return Err("JOINLINK returned an invalid chat room ID".to_string());
    }
    let room = unsafe { find_room(env, chat_id)? };
    if !unsafe { room_has_link(env, resolver, room, link_id)? } {
        return Err("joined open chat link verification failed".to_string());
    }
    Ok(OpenChatJoinResult {
        room_name,
        profile_applied: true,
    })
}

unsafe fn room_has_link(
    env: *mut JNIEnv,
    resolver: jni::sys::jclass,
    room: jni::sys::jobject,
    link_id: i64,
) -> Result<bool, String> {
    unsafe {
        call_static_boolean(
            env,
            resolver,
            "hasLongIdentity",
            "(Ljava/lang/Object;J)Z",
            &[object_value(room), crate::long_value(link_id)],
        )
    }
}

unsafe fn create_open_chat_profile(
    env: *mut JNIEnv,
    profile_id: &str,
    profile_kind: &str,
    nickname: &str,
    profile_image_url: Option<&str>,
) -> Result<jobject, String> {
    match profile_kind {
        "kakao" => {
            let companion = unsafe { signature_object(env, "open-chat-kakao-profile")? };
            let nickname = unsafe { new_string(env, nickname)? };
            let image = unsafe { new_string(env, profile_image_url.unwrap_or_default())? };
            unsafe {
                invoke_signature_operation(
                    env,
                    "create-kakao-profile",
                    companion,
                    &[nickname.cast(), image.cast()],
                )
            }
        }
        "open-profile" => {
            let profile_link_id = profile_id
                .parse::<i64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| "open profile ID must be a positive integer".to_string())?;
            let companion = unsafe { signature_object(env, "open-chat-open-profile")? };
            let use_type_class = unsafe { signature_class(env, "open-chat-profile-use-type")? };
            let enum_class = unsafe { find_class(env, "java/lang/Enum")? };
            let common_name = unsafe { new_string(env, "COMMON")? };
            let common = unsafe {
                call_static_object(
                    env,
                    enum_class,
                    "valueOf",
                    "(Ljava/lang/Class;Ljava/lang/String;)Ljava/lang/Enum;",
                    &[
                        object_value(use_type_class.cast()),
                        object_value(common_name.cast()),
                    ],
                )?
            };
            let profile_link_id = unsafe { box_long(env, profile_link_id)? };
            unsafe {
                invoke_signature_operation(
                    env,
                    "create-open-profile",
                    companion,
                    &[profile_link_id, common],
                )
            }
        }
        _ => Err("unsupported open chat profile kind".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{is_open_link_url, is_open_profile_url};

    #[test]
    fn validates_only_canonical_open_chat_and_profile_urls() {
        assert!(is_open_link_url("https://open.kakao.com/o/example1"));
        assert!(!is_open_link_url("https://open.kakao.com/me/example1"));
        assert!(!is_open_link_url("https://open.kakao.com/o/example?x=1"));

        assert!(is_open_profile_url("https://open.kakao.com/o/example1"));
        assert!(is_open_profile_url("https://open.kakao.com/me/example_1"));
        assert!(!is_open_profile_url(
            "https://open.kakao.com/me/example?x=1"
        ));
    }
}
