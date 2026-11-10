use crate::{Operation, command_operation};
use jni::sys::{JNIEnv, jstring};

pub(crate) unsafe fn dispatch_dialog(env: *mut JNIEnv, id: u64) -> Result<(), String> {
    let Operation::Dialog(command) = command_operation(id)? else {
        return Err("dialog command state mismatch".into());
    };
    let payload = serde_json::to_string(&command).map_err(|e| e.to_string())?;
    let resolver = unsafe { crate::app_class(env, "dev.noa.kakao.OverlayDialogs")? };
    let payload = unsafe { crate::new_string(env, &payload)? };
    let result = unsafe {
        crate::call_static_object(
            env,
            resolver,
            "execute",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[crate::object_value(payload)],
        )?
    };
    if result.is_null() {
        return Err("dialog returned null".into());
    }
    let result = unsafe { crate::java_string(env, result as jstring)? };
    crate::commands::mark_result(id, Ok(Some(result)));
    Ok(())
}
