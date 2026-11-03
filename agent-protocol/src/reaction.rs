use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReactionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction_type: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoticon: Option<EmoticonReaction>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmoticonReaction {
    pub r#type: i32,
    pub id: String,
}

impl ReactionRequest {
    pub fn validate(&self) -> Result<(), String> {
        match (&self.reaction_type, &self.emoticon) {
            (Some(kind), None) if (1..=6).contains(kind) => Ok(()),
            (None, Some(item))
                if (1..=2).contains(&item.r#type)
                    && !item.id.trim().is_empty()
                    && item.id.len() <= 256
                    && !item.id.chars().any(char::is_control) =>
            {
                Ok(())
            }
            _ => Err("Specify reactionType (1-6) or emoticon {type: 1-2, id}".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_exclusive_reaction_selectors() {
        for value in [
            serde_json::json!({"reactionType":1}),
            serde_json::json!({"emoticon":{"type":1,"id":"item"}}),
        ] {
            assert!(
                serde_json::from_value::<ReactionRequest>(value)
                    .unwrap()
                    .validate()
                    .is_ok()
            );
        }
        for value in [
            serde_json::json!({}),
            serde_json::json!({"reactionType":0}),
            serde_json::json!({"reactionType":7}),
            serde_json::json!({"emoticon":{"type":1,"id":""}}),
            serde_json::json!({"reactionType":1,"emoticon":{"type":1,"id":"item"}}),
        ] {
            assert!(
                serde_json::from_value::<ReactionRequest>(value)
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }
}
