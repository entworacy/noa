//! Wire-level channel ownership shared by the host and injected Kakao agent.
pub const VERSION: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Control,
    Vox,
    Audio,
}

impl Channel {
    pub const ALL: [Self; 3] = [Self::Control, Self::Vox, Self::Audio];

    pub const fn index(self) -> usize {
        match self {
            Self::Control => 0,
            Self::Vox => 1,
            Self::Audio => 2,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Vox => "vox",
            Self::Audio => "audio",
        }
    }

    pub fn for_action(action: &str) -> Option<Self> {
        match action {
            "send-custom"
            | "kick-member"
            | "hide-message"
            | "chat-on-room"
            | "load-open-chat-member"
            | "share-open-profile"
            | "join-open-chat" => Some(Self::Control),
            "vox-start-call" | "vox-create-room" | "vox-join-room" | "vox-leave" => Some(Self::Vox),
            "vox-status" | "vox-audio-start" | "vox-audio-push" | "vox-audio-stop" => {
                Some(Self::Audio)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_and_status_do_not_wait_for_room_control() {
        assert_eq!(Channel::for_action("vox-audio-push"), Some(Channel::Audio));
        assert_eq!(Channel::for_action("vox-status"), Some(Channel::Audio));
        assert_eq!(Channel::for_action("vox-join-room"), Some(Channel::Vox));
        assert_eq!(Channel::for_action("send-custom"), Some(Channel::Control));
        assert_eq!(Channel::for_action("unknown"), None);
    }
}
