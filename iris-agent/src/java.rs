//! JNI reflection and value conversion for this adapter.
use jni::sys::{
    JNIEnv, JNINativeMethod, jbyteArray, jclass, jobject, jobjectArray, jstring, jvalue,
};
use std::{
    ffi::{CString, c_void},
    ptr,
};
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

pub(crate) unsafe fn find_method(
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
    let count = unsafe { array_length(env, methods)? };
    for index in 0..count {
        let method = unsafe { array_element(env, methods, index)? };
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
        if unsafe { array_length(env, parameters)? } as usize != parameter_types.len() {
            continue;
        }
        let mut matches = true;
        for (parameter_index, expected) in parameter_types.iter().enumerate() {
            let parameter = unsafe { array_element(env, parameters, parameter_index as i32)? };
            let actual = unsafe { class_name(env, parameter.cast())? };
            if actual != *expected {
                matches = false;
                break;
            }
        }
        if matches {
            unsafe { call_void(env, method, "setAccessible", "(Z)V", &[bool_value(true)])? };
            return Ok(method);
        }
    }
    Err(format!(
        "method {name}({}) was not found",
        parameter_types.join(", ")
    ))
}

pub(crate) unsafe fn class_name(env: *mut JNIEnv, class: jclass) -> Result<String, String> {
    let name = unsafe { call_object(env, class, "getName", "()Ljava/lang/String;", &[])? };
    unsafe { java_string(env, name.cast()) }
}

pub(crate) unsafe fn invoke_reflect(
    env: *mut JNIEnv,
    method: jobject,
    receiver: jobject,
    arguments: &[jobject],
) -> Result<jobject, String> {
    let array = unsafe { object_array(env, arguments)? };
    unsafe {
        call_object(
            env,
            method,
            "invoke",
            "(Ljava/lang/Object;[Ljava/lang/Object;)Ljava/lang/Object;",
            &[object_value(receiver), object_value(array)],
        )
    }
}

pub(crate) unsafe fn static_object(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    signature: &str,
) -> Result<jobject, String> {
    let name = CString::new(name).map_err(|_| "field name contains NUL".to_string())?;
    let signature =
        CString::new(signature).map_err(|_| "field signature contains NUL".to_string())?;
    let field =
        unsafe { ((**env).v1_4.GetStaticFieldID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve static field")? };
    let value = unsafe { ((**env).v1_4.GetStaticObjectField)(env, class, field) };
    unsafe { check(env, "read static field")? };
    if value.is_null() {
        Err("static field is null".to_string())
    } else {
        Ok(value)
    }
}

pub(crate) unsafe fn find_class(env: *mut JNIEnv, name: &str) -> Result<jclass, String> {
    let name = CString::new(name).map_err(|_| "class name contains NUL".to_string())?;
    let class = unsafe { ((**env).v1_4.FindClass)(env, name.as_ptr()) };
    unsafe { check(env, "find Java class")? };
    if class.is_null() {
        Err("Java class is null".to_string())
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
    let signature =
        CString::new(signature).map_err(|_| "constructor signature contains NUL".to_string())?;
    let constructor =
        unsafe { ((**env).v1_4.GetMethodID)(env, class, c"<init>".as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve constructor")? };
    let object = unsafe { ((**env).v1_4.NewObjectA)(env, class, constructor, arguments.as_ptr()) };
    unsafe { check(env, "construct Java object")? };
    if object.is_null() {
        Err("constructed Java object is null".to_string())
    } else {
        Ok(object)
    }
}

pub(crate) unsafe fn call_object(
    env: *mut JNIEnv,
    object: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<jobject, String> {
    let class = unsafe { ((**env).v1_4.GetObjectClass)(env, object) };
    unsafe { check(env, "resolve Java object class")? };
    let name = CString::new(name).map_err(|_| "method name contains NUL".to_string())?;
    let signature =
        CString::new(signature).map_err(|_| "method signature contains NUL".to_string())?;
    let method =
        unsafe { ((**env).v1_4.GetMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve Java method")? };
    let result =
        unsafe { ((**env).v1_4.CallObjectMethodA)(env, object, method, arguments.as_ptr()) };
    unsafe { check(env, "call Java method")? };
    Ok(result)
}

pub(crate) unsafe fn call_static_object(
    env: *mut JNIEnv,
    class: jclass,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<jobject, String> {
    let name = CString::new(name).map_err(|_| "method name contains NUL".to_string())?;
    let signature =
        CString::new(signature).map_err(|_| "method signature contains NUL".to_string())?;
    let method =
        unsafe { ((**env).v1_4.GetStaticMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve static Java method")? };
    let result =
        unsafe { ((**env).v1_4.CallStaticObjectMethodA)(env, class, method, arguments.as_ptr()) };
    unsafe { check(env, "call static Java method")? };
    Ok(result)
}

pub(crate) unsafe fn call_void(
    env: *mut JNIEnv,
    object: jobject,
    name: &str,
    signature: &str,
    arguments: &[jvalue],
) -> Result<(), String> {
    let class = unsafe { ((**env).v1_4.GetObjectClass)(env, object) };
    unsafe { check(env, "resolve Java object class")? };
    let name = CString::new(name).map_err(|_| "method name contains NUL".to_string())?;
    let signature =
        CString::new(signature).map_err(|_| "method signature contains NUL".to_string())?;
    let method =
        unsafe { ((**env).v1_4.GetMethodID)(env, class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check(env, "resolve Java void method")? };
    unsafe { ((**env).v1_4.CallVoidMethodA)(env, object, method, arguments.as_ptr()) };
    unsafe { check(env, "call Java void method") }
}

pub(crate) unsafe fn object_array(
    env: *mut JNIEnv,
    values: &[jobject],
) -> Result<jobjectArray, String> {
    let class = unsafe { find_class(env, "java/lang/Object")? };
    let array =
        unsafe { ((**env).v1_4.NewObjectArray)(env, values.len() as i32, class, ptr::null_mut()) };
    unsafe { check(env, "create object array")? };
    for (index, value) in values.iter().enumerate() {
        unsafe { ((**env).v1_4.SetObjectArrayElement)(env, array, index as i32, *value) };
        unsafe { check(env, "write object array")? };
    }
    Ok(array)
}

pub(crate) unsafe fn array_length(env: *mut JNIEnv, array: jobjectArray) -> Result<i32, String> {
    let length = unsafe { ((**env).v1_4.GetArrayLength)(env, array) };
    unsafe { check(env, "read array length")? };
    Ok(length)
}

pub(crate) unsafe fn array_element(
    env: *mut JNIEnv,
    array: jobjectArray,
    index: i32,
) -> Result<jobject, String> {
    let value = unsafe { ((**env).v1_4.GetObjectArrayElement)(env, array, index) };
    unsafe { check(env, "read array element")? };
    Ok(value)
}

pub(crate) unsafe fn object_text(env: *mut JNIEnv, object: jobject) -> Result<String, String> {
    if object.is_null() {
        return Err("Java object is null".to_string());
    }
    let text = unsafe { call_object(env, object, "toString", "()Ljava/lang/String;", &[])? };
    unsafe { java_string(env, text.cast()) }
}

pub(crate) unsafe fn new_string(env: *mut JNIEnv, value: &str) -> Result<jobject, String> {
    let utf16 = value.encode_utf16().collect::<Vec<_>>();
    let string = unsafe { ((**env).v1_4.NewString)(env, utf16.as_ptr(), utf16.len() as i32) };
    unsafe { check(env, "create Java string")? };
    if string.is_null() {
        Err("Java string is null".to_string())
    } else {
        Ok(string.cast())
    }
}

pub(crate) unsafe fn java_byte_array(
    env: *mut JNIEnv,
    value: jbyteArray,
) -> Result<Vec<u8>, String> {
    if value.is_null() {
        return Err("Java byte array is null".to_string());
    }
    let length = unsafe { ((**env).v1_4.GetArrayLength)(env, value) };
    unsafe { check(env, "read Java byte array length")? };
    let mut bytes = vec![0_u8; length as usize];
    if length > 0 {
        unsafe {
            ((**env).v1_4.GetByteArrayRegion)(env, value, 0, length, bytes.as_mut_ptr().cast())
        };
        unsafe { check(env, "read Java byte array")? };
    }
    Ok(bytes)
}

pub(crate) unsafe fn java_string(env: *mut JNIEnv, value: jstring) -> Result<String, String> {
    if value.is_null() {
        return Err("Java string is null".to_string());
    }
    let length = unsafe { ((**env).v1_4.GetStringLength)(env, value) };
    let chars = unsafe { ((**env).v1_4.GetStringChars)(env, value, ptr::null_mut()) };
    unsafe { check(env, "read Java string")? };
    if chars.is_null() {
        return Err("Java string characters are null".to_string());
    }
    let text =
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(chars, length as usize) });
    unsafe { ((**env).v1_4.ReleaseStringChars)(env, value, chars) };
    Ok(text)
}

pub(crate) unsafe fn check(env: *mut JNIEnv, context: &str) -> Result<(), String> {
    if !unsafe { ((**env).v1_4.ExceptionCheck)(env) } {
        return Ok(());
    }
    let exception = unsafe { ((**env).v1_4.ExceptionOccurred)(env) };
    unsafe { ((**env).v1_4.ExceptionClear)(env) };
    let detail = if exception.is_null() {
        "unknown Java exception".to_string()
    } else {
        unsafe { exception_text(env, exception) }.unwrap_or_else(|_| "Java exception".to_string())
    };
    Err(format!("{context}: {detail}"))
}

pub(crate) unsafe fn exception_text(
    env: *mut JNIEnv,
    exception: jobject,
) -> Result<String, String> {
    let mut current = exception;
    let mut details = Vec::new();
    for _ in 0..6 {
        if current.is_null() {
            break;
        }
        details.push(unsafe { object_text(env, current)? });
        current = unsafe { call_object(env, current, "getCause", "()Ljava/lang/Throwable;", &[])? };
    }
    Ok(details.join(": caused by "))
}

pub(crate) unsafe fn throw_runtime(env: *mut JNIEnv, message: &str) {
    let Ok(class) = (unsafe { find_class(env, "java/lang/RuntimeException") }) else {
        return;
    };
    let message = CString::new(message.replace('\0', " ")).unwrap();
    unsafe { ((**env).v1_4.ThrowNew)(env, class, message.as_ptr()) };
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

pub(crate) fn int_value(value: i32) -> jvalue {
    jvalue { i: value }
}

pub(crate) fn bool_value(value: bool) -> jvalue {
    jvalue { z: value }
}
