use jni::sys::JNIEnv;

use crate::{
    app_class, box_long, call_static_object, find_class, invoke, loco_connected, long_value,
    mark_loaded, new_object, object_text, object_value, static_field, static_object_with_method,
};

pub(crate) unsafe fn send_getmem(
    env: *mut JNIEnv,
    id: u64,
    room: i64,
    user: i64,
) -> Result<(), String> {
    if !unsafe { loco_connected(env)? } {
        return Err("KakaoTalk Loco is not connected".to_string());
    }

    let helper_class = unsafe { app_class(env, "Yr.l0")? };
    let helper = unsafe { static_object_with_method(env, helper_class, "m1", 3)? };
    let room_value = unsafe { box_long(env, room)? };
    let user_value = unsafe { box_long(env, user)? };
    let collections_class = unsafe { find_class(env, "java/util/Collections")? };
    let users = unsafe {
        call_static_object(
            env,
            collections_class,
            "singletonList",
            "(Ljava/lang/Object;)Ljava/util/List;",
            &[object_value(user_value)],
        )?
    };
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
    let result = unsafe { invoke(env, helper, "m1", &[room_value, users, continuation])? };
    if result.is_null() || unsafe { object_text(env, result) }? != "COROUTINE_SUSPENDED" {
        mark_loaded(id);
    }
    Ok(())
}
