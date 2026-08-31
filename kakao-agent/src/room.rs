use jni::sys::{JNIEnv, jclass, jstring};

use crate::{app_class, call_static_void, events, java_string};

pub(crate) unsafe fn install(env: *mut JNIEnv) -> Result<(), String> {
    let watcher = unsafe { app_class(env, "dev.noa.kakao.RoomWatcher")? };
    unsafe { call_static_void(env, watcher, "install", "()V", &[]) }
}

pub(crate) unsafe extern "system" fn invalidated(
    env: *mut JNIEnv,
    _: jclass,
    database: jstring,
    table: jstring,
) {
    if database.is_null() || table.is_null() {
        return;
    }
    let database = unsafe { java_string(env, database) };
    let table = unsafe { java_string(env, table) };
    match (database, table) {
        (Ok(database), Ok(table)) => events::database_invalidated(database, table),
        (Err(error), _) | (_, Err(error)) => {
            crate::log(
                crate::LOG_ERROR,
                &format!("Room invalidation callback failed: {error}"),
            );
        }
    }
}
