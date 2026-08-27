use std::ptr;

use jni::sys::{JNIEnv, jobject};

use crate::{
    OpenChatJoinResult, app_class, box_long, call_boolean, call_object, call_static_object,
    find_class, find_room, invoke, new_object, new_string, object_text, object_value, static_field,
    static_object_with_method, unbox_boolean, unbox_long,
};

unsafe fn cached_open_profile_url(env: *mut JNIEnv, link: i64) -> Result<Option<String>, String> {
    let manager_class = unsafe { app_class(env, "UV.h")? };
    let manager = unsafe { static_field(env, manager_class, "C")? };
    let link_value = unsafe { box_long(env, link)? };
    let open_link = unsafe { invoke(env, manager, "d", &[link_value])? };
    if open_link.is_null() {
        return Ok(None);
    }
    let url_object = unsafe { invoke(env, open_link, "getUrl", &[])? };
    if url_object.is_null() {
        return Ok(None);
    }
    let url = unsafe { object_text(env, url_object)? };
    if !is_open_profile_url(&url) {
        return Ok(None);
    }
    Ok(Some(url))
}

pub(crate) unsafe fn load_open_profile_url(env: *mut JNIEnv, link: i64) -> Result<String, String> {
    if let Some(url) = unsafe { cached_open_profile_url(env, link)? } {
        return Ok(url);
    }
    let repository_class = unsafe { app_class(env, "IW.b")? };
    let repository = unsafe { static_field(env, repository_class, "a")? };
    let link_value = unsafe { box_long(env, link)? };
    let response = unsafe { invoke(env, repository, "e", &[link_value, ptr::null_mut()])? };
    if response.is_null() {
        return Err(format!("open profile response was empty: {link}"));
    }
    let open_link = unsafe { invoke(env, response, "d", &[])? };
    if open_link.is_null() {
        return Err(format!("open profile link was not found: {link}"));
    }
    let url_object = unsafe { invoke(env, open_link, "j", &[])? };
    if url_object.is_null() {
        return Err(format!("open profile URL was not found: {link}"));
    }
    let url = unsafe { object_text(env, url_object)? };
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
    let connection_class = unsafe { app_class(env, "VU.e")? };
    let connection_companion = unsafe { static_object_with_method(env, connection_class, "c", 2)? };
    let url_value = unsafe { new_string(env, url)? };
    let intent = unsafe {
        invoke(
            env,
            connection_companion,
            "c",
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
    let response = unsafe { invoke(env, connection, "f", &[])? };
    if response.is_null() {
        return Err("open chat join information was empty".to_string());
    }
    let loco_open_link = unsafe { invoke(env, response, "d", &[])? };
    if loco_open_link.is_null() {
        return Err("open chat link information was empty".to_string());
    }
    let open_link_class = unsafe { app_class(env, "com.kakao.talk.openlink.db.model.OpenLink")? };
    let open_link_companion = unsafe { static_object_with_method(env, open_link_class, "c", 1)? };
    let open_link = unsafe { invoke(env, open_link_companion, "c", &[loco_open_link])? };
    if open_link.is_null() {
        return Err("open chat link conversion returned empty data".to_string());
    }
    let is_open_profile = unsafe { invoke(env, open_link, "e", &[])? };
    if unsafe { unbox_boolean(env, is_open_profile)? } {
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
    let link_id = unsafe { unbox_long(env, invoke(env, open_link, "w", &[])?)? };
    if link_id <= 0 {
        return Err("open chat link ID was invalid".to_string());
    }

    let manager_class = unsafe { app_class(env, "UV.h")? };
    let manager = unsafe { static_field(env, manager_class, "C")? };
    unsafe { invoke(env, manager, "a0", &[open_link])? };

    let roots = unsafe { app_class(env, "Yr.c1")? };
    let holder = unsafe { static_field(env, roots, "n")? };
    let repository = unsafe { invoke(env, holder, "j", &[])? };
    if let Some(room) = unsafe { find_active_open_chat(env, repository, link_id)? } {
        let actual_link_id = unsafe { unbox_long(env, invoke(env, room, "J0", &[])?)? };
        if actual_link_id != link_id {
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
    unsafe { invoke(env, manager, "Z", &[open_link, profile])? };
    let pre_chat = unsafe {
        invoke(
            env,
            repository,
            "W",
            &[open_link, ptr::null_mut(), ptr::null_mut(), ptr::null_mut()],
        )?
    };
    if pre_chat.is_null() {
        return Err("open chat pre-chat room was not created".to_string());
    }
    let join_class = unsafe { app_class(env, "Yr.l0")? };
    let join_manager = unsafe { static_object_with_method(env, join_class, "s0", 1)? };
    let chat_id = unsafe { unbox_long(env, invoke(env, join_manager, "s0", &[pre_chat])?)? };
    if chat_id <= 0 {
        return Err("JOINLINK returned an invalid chat room ID".to_string());
    }
    let room = unsafe { find_room(env, chat_id)? };
    let actual_link_id = unsafe { unbox_long(env, invoke(env, room, "J0", &[])?)? };
    if actual_link_id != link_id {
        return Err("joined open chat link verification failed".to_string());
    }
    Ok(OpenChatJoinResult {
        room_name,
        profile_applied: true,
    })
}

unsafe fn find_active_open_chat(
    env: *mut JNIEnv,
    repository: jobject,
    link_id: i64,
) -> Result<Option<jobject>, String> {
    let link_value = unsafe { box_long(env, link_id)? };
    let rooms = unsafe { invoke(env, repository, "Y", &[link_value])? };
    if rooms.is_null() {
        return Ok(None);
    }
    let iterator = unsafe { call_object(env, rooms, "iterator", "()Ljava/util/Iterator;", &[])? };
    while unsafe { call_boolean(env, iterator, "hasNext", "()Z", &[])? } {
        let room = unsafe { call_object(env, iterator, "next", "()Ljava/lang/Object;", &[])? };
        if room.is_null() {
            continue;
        }
        let room_id = unsafe { unbox_long(env, invoke(env, room, "q0", &[])?)? };
        let deactivated = unsafe { unbox_boolean(env, invoke(env, room, "S1", &[])?)? };
        if room_id > 0 && !deactivated {
            return Ok(Some(room));
        }
    }
    Ok(None)
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
            let profile_class = unsafe { app_class(env, "yU.z$c")? };
            let companion = unsafe { static_object_with_method(env, profile_class, "a", 2)? };
            let nickname = unsafe { new_string(env, nickname)? };
            let image = unsafe { new_string(env, profile_image_url.unwrap_or_default())? };
            unsafe { invoke(env, companion, "a", &[nickname.cast(), image.cast()]) }
        }
        "open-profile" => {
            let profile_link_id = profile_id
                .parse::<i64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| "open profile ID must be a positive integer".to_string())?;
            let profile_class = unsafe { app_class(env, "yU.z$d")? };
            let companion = unsafe { static_object_with_method(env, profile_class, "a", 2)? };
            let use_type_class = unsafe { app_class(env, "yU.z$d$b")? };
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
            unsafe { invoke(env, companion, "a", &[profile_link_id, common]) }
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
