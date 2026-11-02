use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_BODY_BYTES: usize = 64 * 1024;
pub const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SendLocoRequest {
    pub method: String,
    pub body: Value,
}

impl SendLocoRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.method.is_empty()
            || self.method.len() > 11
            || !self
                .method
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err("method must contain 1–11 uppercase ASCII letters/digits".into());
        }
        if !self.body.is_object() {
            return Err("body must be a JSON object".into());
        }
        if self.body.to_string().len() > MAX_BODY_BYTES {
            return Err("LOCO body exceeds 65536 JSON bytes".into());
        }
        // The root is always a document, never a typed scalar.
        for (key, value) in self.body.as_object().unwrap() {
            key_valid(key)?;
            validate_value(value, 1)?;
        }
        Ok(())
    }
}

fn key_valid(key: &str) -> Result<(), String> {
    if key.contains('\0') {
        Err("BSON keys cannot contain NUL".into())
    } else {
        Ok(())
    }
}

fn validate_value(value: &Value, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("LOCO body nesting exceeds 32 levels".into());
    }
    match value {
        Value::Object(fields) => {
            for key in fields.keys() {
                key_valid(key)?;
            }
            for tag in ["$numberLong", "$numberInt", "$numberDouble", "$binary"] {
                if let Some(value) = fields.get(tag) {
                    if fields.len() != 1 {
                        return Err(format!("{tag} must be the only field in its wrapper"));
                    }
                    let text = value
                        .as_str()
                        .ok_or_else(|| format!("{tag} requires a string"))?;
                    let valid = match tag {
                        "$numberLong" => text.parse::<i64>().is_ok(),
                        "$numberInt" => text.parse::<i32>().is_ok(),
                        "$numberDouble" => text.parse::<f64>().is_ok_and(f64::is_finite),
                        _ => STANDARD.decode(text).is_ok(),
                    };
                    return if valid {
                        Ok(())
                    } else {
                        Err(format!("invalid {tag} value"))
                    };
                }
            }
            for child in fields.values() {
                validate_value(child, depth + 1)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                validate_value(child, depth + 1)?;
            }
        }
        Value::Number(number) if number.is_u64() && number.as_i64().is_none() => {
            return Err("BSON integers must fit in signed 64 bits".into());
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn request(body: Value) -> SendLocoRequest {
        SendLocoRequest {
            method: "PING".into(),
            body,
        }
    }

    #[test]
    fn accepts_documents_and_explicit_bson_types() {
        assert!(request(json!({"id":{"$numberLong":"9223372036854775807"},"i":{"$numberInt":"1"},"d":{"$numberDouble":"1.5"},"bytes":{"$binary":"AAH/"},"list":[null,true,"한글",{},-1,3.5]})).validate().is_ok());
    }
    #[test]
    fn rejects_lossy_or_malformed_values() {
        for body in [
            json!([]),
            json!(null),
            json!({"a":18446744073709551615u64}),
            json!({"a":{"$numberInt":"2147483648"}}),
            json!({"a":{"$numberLong":12}}),
            json!({"a":{"$numberDouble":"NaN"}}),
            json!({"a":{"$binary":"?"}}),
            json!({"a":{"$numberLong":"1","other":2}}),
            json!({"a\u{0}":1}),
        ] {
            assert!(request(body.clone()).validate().is_err(), "{body}");
        }
        for method in ["", "ping", "PING\n", "ABCDEFGHIJKL", "한글"] {
            assert!(
                SendLocoRequest {
                    method: method.into(),
                    body: json!({})
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn bounds_body_size_and_depth() {
        assert!(
            request(json!({"a":"x".repeat(MAX_BODY_BYTES)}))
                .validate()
                .is_err()
        );
        let mut value = json!(null);
        for _ in 0..MAX_DEPTH {
            value = json!([value]);
        }
        assert!(request(json!({"a":value})).validate().is_err());
    }
}
