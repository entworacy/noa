use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DialogContent {
    pub title: String,
    pub message: String,
    #[serde(default = "default_buttons")]
    pub buttons: Vec<DialogButton>,
    #[serde(default = "default_cancelable")]
    pub cancelable: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DialogButton {
    pub id: String,
    pub label: String,
}

fn default_buttons() -> Vec<DialogButton> {
    vec![DialogButton {
        id: "confirm".into(),
        label: "확인".into(),
    }]
}
fn default_cancelable() -> bool {
    true
}
fn default_timeout() -> u32 {
    60
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "lowercase", deny_unknown_fields)]
pub enum DialogCommand {
    Status,
    Show {
        #[serde(rename = "dialogId")]
        dialog_id: String,
        content: DialogContent,
    },
    Get {
        #[serde(rename = "dialogId")]
        dialog_id: String,
    },
    Dismiss {
        #[serde(rename = "dialogId")]
        dialog_id: String,
    },
}

pub fn validate_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("dialog/button ID must contain 1-64 ASCII letters, digits, '-' or '_'".into());
    }
    Ok(())
}

impl DialogContent {
    pub fn validate(&self) -> Result<(), String> {
        if self.title.trim().is_empty()
            || self.title.chars().count() > 128
            || self.message.trim().is_empty()
            || self.message.chars().count() > 4096
        {
            return Err("title and message are required (maximum 128 and 4096 characters)".into());
        }
        if !(1..=3).contains(&self.buttons.len()) || !(1..=600).contains(&self.timeout_seconds) {
            return Err("1-3 buttons and timeoutSeconds from 1 to 600 are required".into());
        }
        let mut ids = std::collections::HashSet::new();
        for button in &self.buttons {
            validate_id(&button.id)?;
            if !ids.insert(&button.id)
                || button.label.trim().is_empty()
                || button.label.chars().count() > 64
            {
                return Err(
                    "button IDs must be unique and labels must contain 1-64 characters".into(),
                );
            }
        }
        Ok(())
    }
}
impl DialogCommand {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Status => Ok(()),
            Self::Show { dialog_id, content } => {
                validate_id(dialog_id)?;
                content.validate()
            }
            Self::Get { dialog_id } | Self::Dismiss { dialog_id } => validate_id(dialog_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn validates_defaults_bounds_and_unique_button_ids() {
        let content: DialogContent =
            serde_json::from_value(json!({"title":"Noa","message":"hello"})).unwrap();
        assert!(content.validate().is_ok());
        assert_eq!(content.buttons[0].id, "confirm");
        assert_eq!(content.timeout_seconds, 60);
        for value in [
            json!({"title":"","message":"x"}),
            json!({"title":"x","message":""}),
            json!({"title":"x","message":"x","timeoutSeconds":0}),
            json!({"title":"x","message":"x","timeoutSeconds":601}),
            json!({"title":"x","message":"x","buttons":[]}),
            json!({"title":"x","message":"x","buttons":[{"id":"a","label":"A"},{"id":"a","label":"B"}]}),
        ] {
            assert!(
                serde_json::from_value::<DialogContent>(value)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
        for id in ["", "a/b", "x\n", "한글"] {
            assert!(validate_id(id).is_err());
        }
    }
}
