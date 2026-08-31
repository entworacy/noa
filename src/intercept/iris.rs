use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;

pub(super) fn endpoint_body(request: &Value) -> Result<Vec<u8>, String> {
    let body = request
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match request.get("bodyEncoding").and_then(Value::as_str) {
        None => Ok(body.as_bytes().to_vec()),
        Some("base64") => STANDARD
            .decode(body)
            .map_err(|error| format!("Iris endpoint body Base64 해석 실패: {error}")),
        Some(encoding) => Err(format!(
            "지원하지 않는 Iris endpoint body encoding입니다: {encoding}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::endpoint_body;

    #[test]
    fn endpoint_body_preserves_arbitrary_binary() {
        let request = serde_json::json!({
            "bodyEncoding": "base64",
            "body": "AP+AQA=="
        });
        assert_eq!(endpoint_body(&request).unwrap(), [0, 255, 128, 64]);
    }

    #[test]
    fn endpoint_body_keeps_legacy_text_compatibility() {
        let request = serde_json::json!({"body": "{\"chatId\":\"123\"}"});
        assert_eq!(endpoint_body(&request).unwrap(), br#"{"chatId":"123"}"#);
    }

    #[test]
    fn endpoint_body_rejects_unknown_encoding() {
        let request = serde_json::json!({"bodyEncoding": "hex", "body": "00"});
        assert!(endpoint_body(&request).is_err());
    }
}
