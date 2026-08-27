use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub user_id: String,
    pub nickname: String,
    pub profile_image_url: Option<String>,
    pub is_mine: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Room {
    pub chat_id: String,
    pub name: String,
    pub room_type: String,
    pub member_count: usize,
    pub members: Vec<Member>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocoPacket {
    pub id: u64,
    pub direction: String,
    pub method: String,
    pub packet_id: i32,
    pub status: i16,
    pub body_length: i32,
    pub body: String,
    pub captured_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Joined,
    Left,
    Kicked,
    NicknameChanged,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Joined => "joined",
            Self::Left => "left",
            Self::Kicked => "kicked",
            Self::NicknameChanged => "nickname_changed",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "joined" => Some(Self::Joined),
            "left" => Some(Self::Left),
            "kicked" => Some(Self::Kicked),
            "nickname_changed" => Some(Self::NicknameChanged),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomEvent {
    pub id: i64,
    pub chat_id: String,
    pub room_name: String,
    pub kind: EventKind,
    pub user_id: String,
    pub nickname: String,
    pub previous_nickname: Option<String>,
    pub occurred_at: i64,
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct NewRoomEvent {
    pub chat_id: i64,
    pub room_name: String,
    pub kind: EventKind,
    pub user_id: i64,
    pub nickname: String,
    pub previous_nickname: Option<String>,
    pub occurred_at: i64,
    pub source: &'static str,
    pub source_id: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct FeedChange {
    pub database_id: i64,
    pub chat_id: i64,
    pub kind: EventKind,
    pub user_id: i64,
    pub nickname: String,
    pub occurred_at: i64,
}
