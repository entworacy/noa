use std::ptr;

use jni::sys::JNIEnv;

use crate::{
    Operation, app_class, box_boolean, box_long, call_boolean, call_long, call_object,
    call_static_object, command_operation, find_class, find_room, invoke, long_value, mark_loaded,
    new_object, new_string, object_text, object_value, static_field, static_object_with_method,
    unbox_long,
};

pub(crate) unsafe fn start_sending_log_load(env: *mut JNIEnv, id: u64) -> Result<(), String> {
    let manager_class = unsafe { app_class(env, "com.kakao.talk.manager.send.sending.b")? };
    let manager = unsafe { static_field(env, manager_class, "a")? };
    let continuation_class = unsafe { app_class(env, "dev.noa.kakao.LoadContinuation")? };
    let context_class = unsafe { app_class(env, "kotlin.coroutines.EmptyCoroutineContext")? };
    let context = unsafe { static_field(env, context_class, "C")? };
    let continuation = unsafe {
        new_object(
            env,
            continuation_class,
            "(JLkotlin/coroutines/CoroutineContext;)V",
            &[long_value(id as i64), object_value(context)],
        )?
    };
    let result = unsafe { invoke(env, manager, "G", &[continuation])? };
    if result.is_null() || unsafe { object_text(env, result) }? != "COROUTINE_SUSPENDED" {
        mark_loaded(id);
    }
    Ok(())
}

pub(crate) unsafe fn dispatch_send(env: *mut JNIEnv, id: u64) -> Result<(), String> {
    let operation = command_operation(id)?;
    let Operation::Send { room, row } = operation else {
        return Err("send command state mismatch".to_string());
    };
    let manager_class = unsafe { app_class(env, "com.kakao.talk.manager.send.sending.b")? };
    let manager = unsafe { static_field(env, manager_class, "a")? };
    let map = unsafe { static_field(env, manager_class, "b")? };
    let key = unsafe { box_long(env, room)? };
    let queue = unsafe {
        call_object(
            env,
            map,
            "get",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            &[object_value(key)],
        )?
    };
    if queue.is_null() {
        return Err(format!("sending log room not loaded: {room}"));
    }
    let iterator = unsafe { call_object(env, queue, "iterator", "()Ljava/util/Iterator;", &[])? };
    let mut entry = ptr::null_mut();
    while unsafe { call_boolean(env, iterator, "hasNext", "()Z", &[])? } {
        let candidate = unsafe { call_object(env, iterator, "next", "()Ljava/lang/Object;", &[])? };
        let value = unsafe { call_long(env, candidate, "getId", "()J", &[])? };
        if value == row {
            entry = candidate;
            break;
        }
    }
    if entry.is_null() {
        return Err(format!("sending log row not loaded: {row}"));
    }
    let room_object = unsafe { find_room(env, room)? };
    unsafe { invoke(env, manager, "P", &[entry])? };
    let request_class =
        unsafe { app_class(env, "com.kakao.talk.manager.send.ChatSendingLogRequest")? };
    let companion = unsafe { static_object_with_method(env, request_class, "u", 5)? };
    let mode_class =
        unsafe { app_class(env, "com.kakao.talk.manager.send.ChatSendingLogRequest$d")? };
    let mode_name = unsafe { new_string(env, "Resend")? };
    let enum_class = unsafe { find_class(env, "java/lang/Enum")? };
    let mode = unsafe {
        call_static_object(
            env,
            enum_class,
            "valueOf",
            "(Ljava/lang/Class;Ljava/lang/String;)Ljava/lang/Enum;",
            &[object_value(mode_class), object_value(mode_name)],
        )?
    };
    let listener_class = unsafe { app_class(env, "dev.noa.kakao.SendListener")? };
    let listener = unsafe { new_object(env, listener_class, "(J)V", &[long_value(id as i64)])? };
    let no = unsafe { box_boolean(env, false)? };
    unsafe {
        invoke(
            env,
            companion,
            "u",
            &[room_object, entry, mode, listener, no],
        )?
    };
    Ok(())
}

pub(crate) unsafe fn dispatch_kick(env: *mut JNIEnv, id: u64) -> Result<(), String> {
    let operation = command_operation(id)?;
    let Operation::Kick { room, user } = operation else {
        return Err("kick command state mismatch".to_string());
    };
    let room_object = unsafe { find_room(env, room)? };
    let user_value = unsafe { box_long(env, user)? };
    let link = unsafe { invoke(env, room_object, "J0", &[])? };
    let link_id = unsafe { unbox_long(env, link)? };
    let members_class = unsafe { app_class(env, "t50.I0")? };
    let members = unsafe { static_field(env, members_class, "a")? };
    let link_value = unsafe { box_long(env, link_id)? };
    let member = unsafe { invoke(env, members, "o", &[user_value, link_value])? };
    if member.is_null() {
        return Err(format!("open chat member not found: {user}"));
    }
    let manager_class = unsafe { app_class(env, "UV.h")? };
    let manager_root = unsafe { static_field(env, manager_class, "C")? };
    let foreground = unsafe { invoke(env, manager_root, "p", &[])? };
    let listener_class = unsafe { app_class(env, "dev.noa.kakao.KickListener")? };
    let listener = unsafe { new_object(env, listener_class, "(J)V", &[long_value(id as i64)])? };
    let no = unsafe { box_boolean(env, false)? };
    unsafe {
        invoke(
            env,
            foreground,
            "f",
            &[room_object, member, no, no, listener],
        )?
    };
    Ok(())
}
