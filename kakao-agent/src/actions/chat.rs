use std::ptr;

use jni::sys::JNIEnv;

use crate::{
    Operation, app_class, box_long, call_boolean, call_long, call_object, call_static_void,
    command_operation, find_room, invoke_signature_operation, long_value, mark_loaded, new_object,
    object_text, object_value, signature_object, signature_static_value, static_field,
};

pub(crate) unsafe fn start_sending_log_load(env: *mut JNIEnv, id: u64) -> Result<(), String> {
    let manager = unsafe { signature_object(env, "sending-log-manager")? };
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
    let result = unsafe {
        invoke_signature_operation(env, "load-sending-log", manager, &[continuation])?
    };
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
    let map = unsafe {
        signature_static_value(env, "sending-log-manager", "java.util.Map")?
    };
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
    let resolver = unsafe { app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    unsafe {
        call_static_void(
            env,
            resolver,
            "resend",
            "(Ljava/lang/Object;Ljava/lang/Object;J)V",
            &[object_value(room_object), object_value(entry), long_value(id as i64)],
        )?
    };
    Ok(())
}

pub(crate) unsafe fn dispatch_kick(env: *mut JNIEnv, id: u64) -> Result<(), String> {
    let operation = command_operation(id)?;
    let Operation::Kick { room, user } = operation else {
        return Err("kick command state mismatch".to_string());
    };
    let resolver = unsafe { app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    unsafe {
        call_static_void(
            env,
            resolver,
            "kick",
            "(JJJ)V",
            &[long_value(room), long_value(user), long_value(id as i64)],
        )?
    };
    Ok(())
}

pub(crate) unsafe fn dispatch_hide_message(env: *mut JNIEnv, id: u64) -> Result<(), String> {
    let operation = command_operation(id)?;
    let Operation::HideMessage {
        room,
        log,
        log_type,
        message,
    } = operation
    else {
        return Err("hide message command state mismatch".to_string());
    };
    let resolver = unsafe { app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    let message = unsafe { crate::new_string(env, &message)? };
    unsafe {
        call_static_void(
            env,
            resolver,
            "hideMessage",
            "(JJIJLjava/lang/String;)V",
            &[
                long_value(room),
                long_value(log),
                crate::int_value(log_type),
                long_value(id as i64),
                object_value(message.cast()),
            ],
        )?
    };
    Ok(())
}
