use serde_json::{Map, Value};
use std::collections::HashSet;

pub(crate) fn selected(
    types: &HashSet<String>,
    reply_type: &str,
    object: &Map<String, Value>,
) -> bool {
    if !types.contains(reply_type) {
        return false;
    }
    // Iris v0.33 accepts named media objects and arrays for file replies.
    // Noa's file extension owns Base64 strings and explicit local paths.
    !(reply_type == "file"
        && !object.contains_key("path")
        && matches!(object.get("data"), Some(Value::Object(_) | Value::Array(_))))
}

pub(crate) fn validate_shape(object: &Map<String, Value>, selected: bool) -> Result<(), String> {
    for key in object.keys() {
        let allowed = matches!(key.as_str(), "type" | "room" | "data" | "threadId")
            || selected && key == "path";
        if !allowed {
            return Err(format!("unknown Iris /reply field: {key}"));
        }
    }
    if !matches!(object.get("room"), Some(Value::String(_))) {
        return Err("Iris /reply room must be a string".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn route(value: Value, enabled: &[&str]) -> Result<bool, String> {
        let object = value.as_object().unwrap();
        let types = enabled.iter().map(|s| s.to_string()).collect();
        let selected = selected(&types, object["type"].as_str().unwrap(), object);
        validate_shape(object, selected)?;
        Ok(selected)
    }

    #[test]
    fn preserves_native_media_payloads() {
        for reply_type in [
            "text",
            "image",
            "image_multiple",
            "media",
            "video",
            "audio",
            "file",
        ] {
            for data in [
                json!({"name":"문서.pdf","data":"YQ=="}),
                json!([{"b64":"YQ=="}]),
            ] {
                assert!(
                    !route(
                        json!({"type":reply_type,"room":"42","data":data,"threadId":"7"}),
                        &["file", "markdown", "custom"]
                    )
                    .unwrap()
                );
            }
        }
    }

    #[test]
    fn retains_noa_file_formats_and_extensions() {
        for body in [
            json!({"type":"file","room":"42","data":"YQ=="}),
            json!({"type":"file","room":"42","path":"/sdcard/a.pdf"}),
            json!({"type":"markdown","room":"42","data":"**text**"}),
            json!({"type":"custom","room":"42","data":{"type":1,"message":"text"}}),
        ] {
            assert!(route(body, &["file", "markdown", "custom"]).unwrap());
        }
    }

    #[test]
    fn respects_disabled_hooks_and_strict_request_fields() {
        assert!(!route(json!({"type":"file","room":"42","data":"YQ=="}), &[]).unwrap());
        assert!(route(json!({"type":"file","room":"42","path":"/sdcard/a"}), &[]).is_err());
        assert!(
            route(
                json!({"type":"video","room":"42","data":{},"extra":1}),
                &["file"]
            )
            .is_err()
        );
        assert!(route(json!({"type":"file","room":42,"data":"YQ=="}), &["file"]).is_err());
    }
}
