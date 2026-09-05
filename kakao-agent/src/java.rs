//! JNI reflection and value conversion for this adapter.
use jni::sys::{JNIEnv, JNINativeMethod, jclass, jobject, jobjectArray, jstring, jvalue};
use std::{
    ffi::{CStr, CString, c_void},
    ptr,
};
pub(crate) unsafe fn find_exact_method(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    parameter_types: &[&str],
) -> Result<jobject, String> {
    let methods = unsafe {
        call_object(
            env,
            class,
            "getDeclaredMethods",
            "()[Ljava/lang/reflect/Method;",
            &[],
        )? as jobjectArray
    };
    let count = unsafe { ((**env).v1_4.GetArrayLength)(env, methods) };
    unsafe { check(env, "read declared methods")? };
    for index in 0..count {
        let method = unsafe { ((**env).v1_4.GetObjectArrayElement)(env, methods, index) };
        unsafe { check(env, "read declared method")? };
        let method_name =
            unsafe { call_object(env, method, "getName", "()Ljava/lang/String;", &[])? };
        if unsafe { java_string(env, method_name.cast())? } != name {
            continue;
        }
        let parameters = unsafe {
            call_object(
                env,
                method,
                "getParameterTypes",
                "()[Ljava/lang/Class;",
                &[],
            )? as jobjectArray
        };
        let parameter_count = unsafe { ((**env).v1_4.GetArrayLength)(env, parameters) };
        if parameter_count as usize != parameter_types.len() {
            continue;
        }
        let mut matched = true;
        for (position, expected) in parameter_types.iter().enumerate() {
            let parameter =
                unsafe { ((**env).v1_4.GetObjectArrayElement)(env, parameters, position as i32) };
            let actual =
                unsafe { call_object(env, parameter, "getName", "()Ljava/lang/String;", &[])? };
            if unsafe { java_string(env, actual.cast())? } != *expected {
                matched = false;
                break;
            }
        }
        if matched {
            unsafe { call_void(env, method, "setAccessible", "(Z)V", &[bool_value(true)])? };
            return Ok(method);
        }
    }
    Err(format!(
        "method {name}({}) was not found",
        parameter_types.join(", ")
    ))
}

pub(crate) unsafe fn load_class(
    env: *mut JNIEnv,
    loader: jobject,
    name: &str,
) -> Result<jclass, String> {
    let name = unsafe { new_string(env, name)? };
    let class = unsafe {
        call_object(
            env,
            loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[object_value(name)],
        )?
    };
    if class.is_null() {
        Err("class loader returned null".to_string())
    } else {
        Ok(class.cast())
    }
}

pub(crate) unsafe fn static_field(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
) -> Result<jobject, String> {
    let name = unsafe { new_string(env, name)? };
    let field = unsafe {
        call_object(
            env,
            class,
            "getDeclaredField",
            "(Ljava/lang/String;)Ljava/lang/reflect/Field;",
            &[object_value(name)],
        )?
    };
    unsafe { call_void(env, field, "setAccessible", "(Z)V", &[bool_value(true)])? };
    unsafe {
        call_object(
            env,
            field,
            "get",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            &[object_value(ptr::null_mut())],
        )
    }
}

pub(crate) unsafe fn instance_field(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
) -> Result<jobject, String> {
    let mut class = unsafe { ((**env).v1_4.GetObjectClass)(env, target) };
    unsafe { check(env, "resolve instance class")? };
    while !class.is_null() {
        let field_name = unsafe { new_string(env, name)? };
        let field = unsafe {
            call_object(
                env,
                class,
                "getDeclaredField",
                "(Ljava/lang/String;)Ljava/lang/reflect/Field;",
                &[object_value(field_name)],
            )
        };
        match field {
            Ok(field) => {
                unsafe { call_void(env, field, "setAccessible", "(Z)V", &[bool_value(true)])? };
                return unsafe {
                    call_object(
                        env,
                        field,
                        "get",
                        "(Ljava/lang/Object;)Ljava/lang/Object;",
                        &[object_value(target)],
                    )
                };
            }
            Err(_) => {
                class = unsafe {
                    call_object(env, class, "getSuperclass", "()Ljava/lang/Class;", &[])?
                }
                .cast();
            }
        }
    }
    Err(format!("instance field {name} was not found"))
}

pub(crate) unsafe fn instance_field_by_type(
    env: *mut JNIEnv,
    target: jobject,
    type_name: &str,
) -> Result<jobject, String> {
    let mut class = unsafe { ((**env).v1_4.GetObjectClass)(env, target) };
    unsafe { check(env, "resolve instance class")? };
    while !class.is_null() {
        let fields = unsafe {
            call_object(
                env,
                class,
                "getDeclaredFields",
                "()[Ljava/lang/reflect/Field;",
                &[],
            )? as jobjectArray
        };
        let count = unsafe { ((**env).v1_4.GetArrayLength)(env, fields) };
        unsafe { check(env, "read instance fields")? };
        for index in 0..count {
            let field = unsafe { ((**env).v1_4.GetObjectArrayElement)(env, fields, index) };
            unsafe { check(env, "read instance field")? };
            let field_type =
                unsafe { call_object(env, field, "getType", "()Ljava/lang/Class;", &[])? };
            let name =
                unsafe { call_object(env, field_type, "getName", "()Ljava/lang/String;", &[])? };
            if unsafe { java_string(env, name.cast())? } != type_name {
                continue;
            }
            unsafe { call_void(env, field, "setAccessible", "(Z)V", &[bool_value(true)])? };
            return unsafe {
                call_object(
                    env,
                    field,
                    "get",
                    "(Ljava/lang/Object;)Ljava/lang/Object;",
                    &[object_value(target)],
                )
            };
        }
        class =
            unsafe { call_object(env, class, "getSuperclass", "()Ljava/lang/Class;", &[])? }.cast();
    }
    Err(format!("instance field of type {type_name} was not found"))
}

pub(crate) unsafe fn invoke(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    arguments: &[jobject],
) -> Result<jobject, String> {
    if target.is_null() {
        return Err(format!("{name} target is null"));
    }
    let class = unsafe { ((**env).v1_4.GetObjectClass)(env, target) };
    unsafe { check(env, "resolve target class")? };
    let method = unsafe { find_method(env, class, name, arguments.len() as i32)? }
        .ok_or_else(|| format!("method {name}/{} was not found", arguments.len()))?;
    let array = unsafe { object_array(env, arguments)? };
    unsafe {
        call_object(
            env,
            method,
            "invoke",
            "(Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;",
            &[object_value(target), object_value(array)],
        )
    }
}

pub(crate) unsafe fn find_method(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    arity: i32,
) -> Result<Option<jobject>, String> {
    let methods = unsafe {
        call_object(
            env,
            class,
            "getDeclaredMethods",
            "()[Ljava/lang/reflect/Method;",
            &[],
        )? as jobjectArray
    };
    let count = unsafe { ((**env).v1_4.GetArrayLength)(env, methods) };
    unsafe { check(env, "read methods")? };
    for index in 0..count {
        let method = unsafe { ((**env).v1_4.GetObjectArrayElement)(env, methods, index) };
        unsafe { check(env, "read method")? };
        let method_name =
            unsafe { call_object(env, method, "getName", "()Ljava/lang/String;", &[])? };
        if unsafe { java_string(env, method_name.cast())? } != name {
            continue;
        }
        let parameter_count = unsafe { call_int(env, method, "getParameterCount", "()I", &[])? };
        if parameter_count != arity {
            continue;
        }
        unsafe { call_void(env, method, "setAccessible", "(Z)V", &[bool_value(true)])? };
        return Ok(Some(method));
    }
    Ok(None)
}

pub(crate) unsafe fn object_array(env: *mut JNIEnv, values: &[jobject]) -> Result<jobject, String> {
    let class = unsafe { find_class(env, "java/lang/Object")? };
    let array =
        unsafe { ((**env).v1_4.NewObjectArray)(env, values.len() as i32, class, ptr::null_mut()) };
    unsafe { check(env, "create argument array")? };
    for (index, value) in values.iter().enumerate() {
        unsafe { ((**env).v1_4.SetObjectArrayElement)(env, array, index as i32, *value) };
        unsafe { check(env, "fill argument array")? };
    }
    Ok(array.cast())
}

pub(crate) unsafe fn box_long(env: *mut JNIEnv, value: i64) -> Result<jobject, String> {
    let class = unsafe { find_class(env, "java/lang/Long")? };
    unsafe {
        call_static_object(
            env,
            class,
            "valueOf",
            "(J)Ljava/lang/Long;",
            &[long_value(value)],
        )
    }
}

pub(crate) unsafe fn unbox_long(env: *mut JNIEnv, value: jobject) -> Result<i64, String> {
    unsafe { call_long(env, value, "longValue", "()J", &[]) }
}

pub(crate) unsafe fn unbox_number(env: *mut JNIEnv, value: jobject) -> Result<i32, String> {
    unsafe { call_int(env, value, "intValue", "()I", &[]) }
}

pub(crate) unsafe fn object_text(env: *mut JNIEnv, value: jobject) -> Result<String, String> {
    let text = unsafe { call_object(env, value, "toString", "()Ljava/lang/String;", &[])? };
    unsafe { java_string(env, text.cast()) }
}

pub(crate) unsafe fn find_class(env: *mut JNIEnv, name: &str) -> Result<jclass, String> {
    let name = CString::new(name).map_err(|_| "class name contains NUL".to_string())?;
    let class = unsafe { ((**env).v1_4.FindClass)(env, name.as_ptr()) };
    unsafe { check(env, "find class")? };
    if class.is_null() {
        Err("class was not found".to_string())
    } else {
        Ok(class)
    }
}

pub(crate) unsafe fn new_object(
    env: *mut JNIEnv,
    class: jclass,
    signature: &str,
    arguments: &[jvalue],
) -> Result<jobject, String> {
    let name = c"<init>";
    let signature = CString::new(signature).unwrap();
    let method =
        unsafe { ((**env).v1_4.GetMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve constructor")? };
    if method.is_null() {
        return Err("constructor was not found".to_string());
    }
    let value = unsafe { ((**env).v1_4.NewObjectA)(env, class, method, arguments.as_ptr()) };
    unsafe { check(env, "construct object")? };
    if value.is_null() {
        Err("constructor returned null".to_string())
    } else {
        Ok(value)
    }
}

pub(crate) unsafe fn call_object(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<jobject, String> {
    let method = unsafe { method_id(env, target, name, signature)? };
    let value =
        unsafe { ((**env).v1_4.CallObjectMethodA)(env, target, method, arguments.as_ptr()) };
    unsafe { check(env, name)? };
    Ok(value)
}

pub(crate) unsafe fn call_boolean(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<bool, String> {
    let method = unsafe { method_id(env, target, name, signature)? };
    let value =
        unsafe { ((**env).v1_4.CallBooleanMethodA)(env, target, method, arguments.as_ptr()) };
    unsafe { check(env, name)? };
    Ok(value)
}

pub(crate) unsafe fn call_int(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<i32, String> {
    let method = unsafe { method_id(env, target, name, signature)? };
    let value = unsafe { ((**env).v1_4.CallIntMethodA)(env, target, method, arguments.as_ptr()) };
    unsafe { check(env, name)? };
    Ok(value)
}

pub(crate) unsafe fn call_long(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<i64, String> {
    let method = unsafe { method_id(env, target, name, signature)? };
    let value = unsafe { ((**env).v1_4.CallLongMethodA)(env, target, method, arguments.as_ptr()) };
    unsafe { check(env, name)? };
    Ok(value)
}

pub(crate) unsafe fn call_void(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<(), String> {
    let method = unsafe { method_id(env, target, name, signature)? };
    unsafe { ((**env).v1_4.CallVoidMethodA)(env, target, method, arguments.as_ptr()) };
    unsafe { check(env, name) }
}

pub(crate) unsafe fn method_id(
    env: *mut JNIEnv,
    target: jobject,
    name: &str,
    signature: &str,
) -> Result<*mut jni::sys::_jmethodID, String> {
    let class = unsafe { ((**env).v1_4.GetObjectClass)(env, target) };
    unsafe { check(env, "resolve object class")? };
    let name = CString::new(name).unwrap();
    let signature = CString::new(signature).unwrap();
    let method =
        unsafe { ((**env).v1_4.GetMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve method")? };
    if method.is_null() {
        Err("method was not found".to_string())
    } else {
        Ok(method)
    }
}

pub(crate) unsafe fn call_static_object(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<jobject, String> {
    let name = CString::new(name).unwrap();
    let signature = CString::new(signature).unwrap();
    let method =
        unsafe { ((**env).v1_4.GetStaticMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve static method")? };
    if method.is_null() {
        return Err("static method was not found".to_string());
    }
    let value =
        unsafe { ((**env).v1_4.CallStaticObjectMethodA)(env, class, method, arguments.as_ptr()) };
    unsafe { check(env, name.to_string_lossy().as_ref())? };
    Ok(value)
}

pub(crate) unsafe fn call_static_boolean(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<bool, String> {
    let name = CString::new(name).unwrap();
    let signature = CString::new(signature).unwrap();
    let method =
        unsafe { ((**env).v1_4.GetStaticMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve static method")? };
    if method.is_null() {
        return Err("static method was not found".to_string());
    }
    let value =
        unsafe { ((**env).v1_4.CallStaticBooleanMethodA)(env, class, method, arguments.as_ptr()) };
    unsafe { check(env, name.to_string_lossy().as_ref())? };
    Ok(value)
}

pub(crate) unsafe fn call_static_void(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<(), String> {
    let name = CString::new(name).unwrap();
    let signature = CString::new(signature).unwrap();
    let method =
        unsafe { ((**env).v1_4.GetStaticMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve static method")? };
    if method.is_null() {
        return Err("static method was not found".to_string());
    }
    unsafe { ((**env).v1_4.CallStaticVoidMethodA)(env, class, method, arguments.as_ptr()) };
    unsafe { check(env, name.to_string_lossy().as_ref()) }
}

pub(crate) unsafe fn new_string(env: *mut JNIEnv, value: &str) -> Result<jstring, String> {
    let value = CString::new(value).map_err(|_| "string contains NUL".to_string())?;
    let string = unsafe { ((**env).v1_4.NewStringUTF)(env, value.as_ptr()) };
    unsafe { check(env, "create string")? };
    if string.is_null() {
        Err("string allocation failed".to_string())
    } else {
        Ok(string)
    }
}

pub(crate) unsafe fn java_string(env: *mut JNIEnv, value: jstring) -> Result<String, String> {
    if value.is_null() {
        return Ok(String::new());
    }
    let pointer = unsafe { ((**env).v1_4.GetStringUTFChars)(env, value, ptr::null_mut()) };
    unsafe { check(env, "read string")? };
    if pointer.is_null() {
        return Err("string characters are null".to_string());
    }
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    unsafe { ((**env).v1_4.ReleaseStringUTFChars)(env, value, pointer) };
    Ok(text)
}

pub(crate) unsafe fn check(env: *mut JNIEnv, context: &str) -> Result<(), String> {
    if !unsafe { ((**env).v1_4.ExceptionCheck)(env) } {
        return Ok(());
    }
    let exception = unsafe { ((**env).v1_4.ExceptionOccurred)(env) };
    unsafe { ((**env).v1_4.ExceptionClear)(env) };
    if exception.is_null() {
        return Err(format!("{context}: Java exception"));
    }
    let mut detail_source = exception;
    let exception_class = unsafe { ((**env).v1_4.GetObjectClass)(env, exception) };
    let cause_method = unsafe {
        ((**env).v1_4.GetMethodID)(
            env,
            exception_class,
            c"getCause".as_ptr(),
            c"()Ljava/lang/Throwable;".as_ptr(),
        )
    };
    if !cause_method.is_null() {
        let cause =
            unsafe { ((**env).v1_4.CallObjectMethodA)(env, exception, cause_method, ptr::null()) };
        if !cause.is_null() && !unsafe { ((**env).v1_4.ExceptionCheck)(env) } {
            detail_source = cause;
        } else {
            unsafe { ((**env).v1_4.ExceptionClear)(env) };
        }
    }
    let class = unsafe { ((**env).v1_4.GetObjectClass)(env, detail_source) };
    let method = unsafe {
        ((**env).v1_4.GetMethodID)(
            env,
            class,
            c"toString".as_ptr(),
            c"()Ljava/lang/String;".as_ptr(),
        )
    };
    if method.is_null() {
        return Err(format!("{context}: Java exception"));
    }
    let value =
        unsafe { ((**env).v1_4.CallObjectMethodA)(env, detail_source, method, ptr::null()) };
    if value.is_null() || unsafe { ((**env).v1_4.ExceptionCheck)(env) } {
        unsafe { ((**env).v1_4.ExceptionClear)(env) };
        return Err(format!("{context}: Java exception"));
    }
    let detail =
        unsafe { java_string(env, value.cast()) }.unwrap_or_else(|_| "Java exception".to_string());
    Err(format!("{context}: {detail}"))
}

pub(crate) fn native_method(name: &str, signature: &str, function: *mut c_void) -> JNINativeMethod {
    JNINativeMethod {
        name: CString::new(name).unwrap().into_raw(),
        signature: CString::new(signature).unwrap().into_raw(),
        fnPtr: function,
    }
}

pub(crate) fn object_value(value: jobject) -> jvalue {
    jvalue { l: value }
}

pub(crate) fn long_value(value: i64) -> jvalue {
    jvalue { j: value }
}

pub(crate) fn int_value(value: i32) -> jvalue {
    jvalue { i: value }
}

pub(crate) fn bool_value(value: bool) -> jvalue {
    jvalue { z: value }
}
