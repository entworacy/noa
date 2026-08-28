use jni::sys::{JNIEnv, jclass, jobject};

use crate::{app_class, call_static_object, java_string, new_string, object_array, object_value};

pub(crate) unsafe fn verify_discovery(env: *mut JNIEnv) -> Result<String, String> {
    let resolver = unsafe { app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    let description = unsafe {
        call_static_object(
            env,
            resolver,
            "verifySignatures",
            "()Ljava/lang/String;",
            &[],
        )?
    };
    unsafe { java_string(env, description.cast()) }
}

pub(crate) unsafe fn signature_class(env: *mut JNIEnv, role: &str) -> Result<jclass, String> {
    let resolver = unsafe { app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    let role = unsafe { new_string(env, role)? };
    let class = unsafe {
        call_static_object(
            env,
            resolver,
            "classFor",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[object_value(role)],
        )?
    };
    if class.is_null() {
        Err("signature resolver returned a null class".to_string())
    } else {
        Ok(class.cast())
    }
}

pub(crate) unsafe fn signature_object(env: *mut JNIEnv, role: &str) -> Result<jobject, String> {
    let resolver = unsafe { app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    let role = unsafe { new_string(env, role)? };
    unsafe {
        call_static_object(
            env,
            resolver,
            "objectFor",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[object_value(role)],
        )
    }
}

pub(crate) unsafe fn signature_static_value(
    env: *mut JNIEnv,
    role: &str,
    type_name: &str,
) -> Result<jobject, String> {
    let resolver = unsafe { app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    let role = unsafe { new_string(env, role)? };
    let type_name = unsafe { new_string(env, type_name)? };
    unsafe {
        call_static_object(
            env,
            resolver,
            "staticValueFor",
            "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/Object;",
            &[object_value(role), object_value(type_name)],
        )
    }
}

pub(crate) unsafe fn invoke_signature_operation(
    env: *mut JNIEnv,
    operation: &str,
    target: jobject,
    arguments: &[jobject],
) -> Result<jobject, String> {
    let resolver = unsafe { app_class(env, "dev.noa.kakao.KakaoSignatureResolver")? };
    let operation = unsafe { new_string(env, operation)? };
    let arguments = unsafe { object_array(env, arguments)? };
    unsafe {
        call_static_object(
            env,
            resolver,
            "invokeOperation",
            "(Ljava/lang/String;Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;",
            &[
                object_value(operation),
                object_value(target),
                object_value(arguments.cast()),
            ],
        )
    }
}
