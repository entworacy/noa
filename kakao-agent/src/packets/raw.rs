use base64::{Engine, engine::general_purpose::STANDARD};
use jni::sys::{JNIEnv, jlongArray, jobject, jvalue};
use noa_agent_protocol::loco::SendLocoRequest;
use serde_json::Value;
use std::ptr;

use crate::{
    app_class, call_boolean, call_object, call_static_object, check, find_class, int_value,
    long_value, new_object, new_string, object_value,
};

pub(crate) unsafe fn send_loco(
    env: *mut JNIEnv,
    packet: &SendLocoRequest,
) -> Result<String, String> {
    packet.validate()?;
    unsafe {
        let body = document(env, packet.body.as_object().unwrap())?;
        let method = new_string(env, &packet.method)?;
        let sender = app_class(env, "dev.noa.kakao.LocoSender")?;
        let result = call_static_object(
            env,
            sender,
            "send",
            "(Ljava/lang/String;Ljava/util/Map;)[J",
            &[object_value(method), object_value(body)],
        )? as jlongArray;
        if result.is_null() || ((**env).v1_4.GetArrayLength)(env, result) != 2 {
            return Err("LOCO sender returned an invalid result; delivery is uncertain".into());
        }
        let mut fields = [0i64; 2];
        ((**env).v1_4.GetLongArrayRegion)(env, result, 0, 2, fields.as_mut_ptr());
        check(env, "read LOCO send result")?;
        Ok(
            serde_json::json!({"ok":true, "method":packet.method, "packetId":fields[0],
            "bodyLength":fields[1], "verification":"socket-write", "serverConfirmed":false})
            .to_string(),
        )
    }
}

// Each child/entry gets a frame so a large document cannot exhaust JNI local refs.
unsafe fn framed(
    env: *mut JNIEnv,
    f: impl FnOnce() -> Result<jobject, String>,
) -> Result<jobject, String> {
    unsafe {
        if ((**env).v1_4.PushLocalFrame)(env, 32) < 0 {
            check(env, "allocate LOCO JSON local frame")?;
            return Err("cannot allocate JNI local frame".into());
        }
        let result = f();
        let value =
            ((**env).v1_4.PopLocalFrame)(env, result.as_ref().copied().unwrap_or(ptr::null_mut()));
        result.map(|_| value)
    }
}

unsafe fn construct(
    env: *mut JNIEnv,
    name: &str,
    signature: &str,
    args: &[jvalue],
) -> Result<jobject, String> {
    unsafe { new_object(env, find_class(env, name)?, signature, args) }
}

unsafe fn document(
    env: *mut JNIEnv,
    fields: &serde_json::Map<String, Value>,
) -> Result<jobject, String> {
    unsafe {
        framed(env, || {
            let map = construct(env, "java/util/LinkedHashMap", "()V", &[])?;
            for (key, value) in fields {
                framed(env, || {
                    let key = unicode_string(env, key)?;
                    let value = convert(env, value)?;
                    call_object(
                        env,
                        map,
                        "put",
                        "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                        &[object_value(key), object_value(value)],
                    )?;
                    Ok(ptr::null_mut())
                })?;
            }
            Ok(map)
        })
    }
}

unsafe fn convert(env: *mut JNIEnv, value: &Value) -> Result<jobject, String> {
    unsafe {
        framed(env, || match value {
            Value::Null => Ok(ptr::null_mut()),
            Value::Bool(value) => construct(
                env,
                "java/lang/Boolean",
                "(Z)V",
                &[crate::bool_value(*value)],
            ),
            Value::String(value) => unicode_string(env, value),
            Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    if let Ok(value) = i32::try_from(value) {
                        construct(env, "java/lang/Integer", "(I)V", &[int_value(value)])
                    } else {
                        construct(env, "java/lang/Long", "(J)V", &[long_value(value)])
                    }
                } else {
                    construct(
                        env,
                        "java/lang/Double",
                        "(D)V",
                        &[jvalue {
                            d: value.as_f64().ok_or("invalid BSON number")?,
                        }],
                    )
                }
            }
            Value::Array(values) => {
                let list = construct(env, "java/util/ArrayList", "()V", &[])?;
                for value in values {
                    framed(env, || {
                        let value = convert(env, value)?;
                        call_boolean(
                            env,
                            list,
                            "add",
                            "(Ljava/lang/Object;)Z",
                            &[object_value(value)],
                        )?;
                        Ok(ptr::null_mut())
                    })?;
                }
                Ok(list)
            }
            Value::Object(fields) => {
                if let Some(value) = fields.get("$numberLong").and_then(Value::as_str) {
                    return construct(
                        env,
                        "java/lang/Long",
                        "(J)V",
                        &[long_value(value.parse::<i64>().map_err(|e| e.to_string())?)],
                    );
                }
                if let Some(value) = fields.get("$numberInt").and_then(Value::as_str) {
                    return construct(
                        env,
                        "java/lang/Integer",
                        "(I)V",
                        &[int_value(value.parse::<i32>().map_err(|e| e.to_string())?)],
                    );
                }
                if let Some(value) = fields.get("$numberDouble").and_then(Value::as_str) {
                    return construct(
                        env,
                        "java/lang/Double",
                        "(D)V",
                        &[jvalue {
                            d: value.parse::<f64>().map_err(|e| e.to_string())?,
                        }],
                    );
                }
                if let Some(value) = fields.get("$binary").and_then(Value::as_str) {
                    let bytes = STANDARD.decode(value).map_err(|e| e.to_string())?;
                    let array = ((**env).v1_4.NewByteArray)(env, bytes.len() as i32);
                    check(env, "allocate BSON binary")?;
                    ((**env).v1_4.SetByteArrayRegion)(
                        env,
                        array,
                        0,
                        bytes.len() as i32,
                        bytes.as_ptr().cast(),
                    );
                    check(env, "copy BSON binary")?;
                    return Ok(array);
                }
                document(env, fields)
            }
        })
    }
}

unsafe fn unicode_string(env: *mut JNIEnv, value: &str) -> Result<jobject, String> {
    let units: Vec<u16> = value.encode_utf16().collect();
    let string = unsafe { ((**env).v1_4.NewString)(env, units.as_ptr(), units.len() as i32) };
    unsafe {
        check(env, "create BSON UTF-16 string")?;
    }
    if string.is_null() {
        Err("BSON string allocation failed".into())
    } else {
        Ok(string)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use jni::sys::{JNI_VERSION_1_8, JavaVM, JavaVMInitArgs, JavaVMOption};
    use std::ffi::{CString, c_void};

    #[link(name = "dl")]
    unsafe extern "C" {
        fn dlopen(path: *const std::ffi::c_char, flags: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
    }

    #[test]
    #[ignore = "requires NOA_TEST_LIBJVM pointing to a local JDK lib/server/libjvm.so"]
    fn json_to_java_preserves_bson_types_and_unicode() {
        unsafe {
            let path = CString::new(std::env::var("NOA_TEST_LIBJVM").unwrap()).unwrap();
            let library = dlopen(path.as_ptr(), 2);
            assert!(!library.is_null(), "cannot load libjvm");
            let symbol = dlsym(library, c"JNI_CreateJavaVM".as_ptr());
            assert!(!symbol.is_null());
            let create: unsafe extern "C" fn(
                *mut *mut JavaVM,
                *mut *mut JNIEnv,
                *mut JavaVMInitArgs,
            ) -> i32 = std::mem::transmute(symbol);
            let mut option = JavaVMOption {
                optionString: c"-Xcheck:jni".as_ptr().cast_mut(),
                extraInfo: ptr::null_mut(),
            };
            let mut args = JavaVMInitArgs {
                version: JNI_VERSION_1_8,
                nOptions: 1,
                options: &mut option,
                ignoreUnrecognized: false,
            };
            let mut vm = ptr::null_mut();
            let mut env = ptr::null_mut();
            assert_eq!(create(&mut vm, &mut env, &mut args), 0);
            let input = serde_json::json!({
                "id":{"$numberLong":"9223372036854775807"},
                "smallLong":{"$numberLong":"1"}, "int":1, "large":2147483648i64,
                "double":{"$numberDouble":"1.5"}, "binary":{"$binary":"AAH/"},
                "한😀":"a\u{0}한😀", "null":null, "bool":true,
                "list":[{"n":{"$numberInt":"-2147483648"}},false],
                "many":vec!["한😀";3000],
            });
            let packet = SendLocoRequest {
                method: "PING".into(),
                body: input.clone(),
            };
            packet.validate().unwrap();
            let map = document(env, input.as_object().unwrap()).unwrap();
            let get = |key: &str| {
                let key = unicode_string(env, key).unwrap();
                call_object(
                    env,
                    map,
                    "get",
                    "(Ljava/lang/Object;)Ljava/lang/Object;",
                    &[object_value(key)],
                )
                .unwrap()
            };
            let class_name = |object| {
                let class =
                    call_object(env, object, "getClass", "()Ljava/lang/Class;", &[]).unwrap();
                let name = call_object(env, class, "getName", "()Ljava/lang/String;", &[]).unwrap();
                crate::java_string(env, name).unwrap()
            };
            assert_eq!(class_name(get("id")), "java.lang.Long");
            assert_eq!(crate::unbox_long(env, get("id")).unwrap(), i64::MAX);
            assert_eq!(class_name(get("smallLong")), "java.lang.Long");
            assert_eq!(class_name(get("int")), "java.lang.Integer");
            assert_eq!(class_name(get("large")), "java.lang.Long");
            assert_eq!(class_name(get("double")), "java.lang.Double");
            assert_eq!(crate::object_text(env, get("double")).unwrap(), "1.5");
            assert_eq!(class_name(get("bool")), "java.lang.Boolean");
            assert!(get("null").is_null());
            let string = get("한😀");
            let count = ((**env).v1_4.GetStringLength)(env, string);
            let mut units = vec![0u16; count as usize];
            ((**env).v1_4.GetStringRegion)(env, string, 0, count, units.as_mut_ptr());
            check(env, "read Unicode fixture").unwrap();
            assert_eq!(String::from_utf16(&units).unwrap(), "a\u{0}한😀");
            let binary = get("binary");
            let mut bytes = [0i8; 3];
            assert_eq!(((**env).v1_4.GetArrayLength)(env, binary), 3);
            ((**env).v1_4.GetByteArrayRegion)(env, binary, 0, 3, bytes.as_mut_ptr());
            check(env, "read binary fixture").unwrap();
            assert_eq!(bytes, [0, 1, -1]);
            let list = get("many");
            let array = call_object(env, list, "toArray", "()[Ljava/lang/Object;", &[]).unwrap();
            assert_eq!(((**env).v1_4.GetArrayLength)(env, array), 3000);
            check(env, "read large list fixture").unwrap();
        }
    }
}
