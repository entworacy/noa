use jni::sys::JNIEnv;

use crate::{
    app_class, find_room, invoke_signature_operation, loco_connected, long_value, mark_complete,
    new_object, object_value, signature_object, static_field,
};

pub(crate) unsafe fn send_chat_on_room(env: *mut JNIEnv, id: u64, room: i64) -> Result<(), String> {
    if !unsafe { loco_connected(env)? } {
        mark_complete(id, Ok(()));
        return Ok(());
    }

    let room_object = unsafe { find_room(env, room)? };
    let helper = unsafe { signature_object(env, "room-api")? };
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
    unsafe {
        invoke_signature_operation(
            env,
            "chat-on-room",
            helper,
            &[room_object, continuation],
        )?
    };
    mark_complete(id, Ok(()));
    Ok(())
}
