use crate::model::LocoPacket;

pub(super) fn kick_failure_detail(room_id: i64, user_id: i64, since: i64) -> Option<String> {
    kick_failure_from(&super::loco_packets(64), room_id, user_id, since)
}

fn kick_failure_from(
    packets: &[LocoPacket],
    room_id: i64,
    user_id: i64,
    since: i64,
) -> Option<String> {
    let room = room_id.to_string();
    let user = user_id.to_string();
    let request = packets.iter().find(|packet| {
        packet.direction == "send"
            && packet.method == "KICKMEM"
            && packet.captured_at >= since
            && body_field(&packet.body, "c") == Some(room.as_str())
            && body_field(&packet.body, "mid") == Some(user.as_str())
    })?;
    let response = packets.iter().find(|packet| {
        packet.direction == "receive"
            && packet.method == "KICKMEM"
            && packet.packet_id == request.packet_id
            && packet.captured_at >= request.captured_at
    })?;
    let body_status =
        body_field(&response.body, "status").and_then(|value| value.parse::<i32>().ok());
    let message = error_message(&response.body);
    if response.status == 0 && body_status.is_none_or(|status| status == 0) && message.is_none() {
        return None;
    }
    let status = body_status
        .map(|status| status.to_string())
        .unwrap_or_else(|| response.status.to_string());
    let reason = message.unwrap_or_else(|| response.body.trim().to_string());
    Some(format!(
        "카카오 서버가 강퇴 요청을 거부했습니다 (status={status}): {reason}"
    ))
}

fn body_field<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}=");
    body.trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .split(", ")
        .find_map(|field| field.strip_prefix(&prefix))
}

fn error_message(body: &str) -> Option<String> {
    let marker = "errMsg=";
    let start = body.find(marker)? + marker.len();
    let message = body[start..].trim().trim_end_matches('}').trim();
    (!message.is_empty()).then(|| message.to_string())
}

#[cfg(test)]
mod tests {
    use super::kick_failure_from;
    use crate::model::LocoPacket;

    #[test]
    fn kick_failure_uses_the_matching_response_reason() {
        let packets = vec![
            loco_packet(
                "receive",
                1892,
                "{status=-500, errMsg=You can't remove a user from a chatroom.}",
                102,
            ),
            loco_packet("send", 1892, "{li=78345137, c=10, mid=20, r=false}", 101),
            loco_packet("receive", 1800, "{status=-500, errMsg=unrelated}", 100),
        ];

        assert_eq!(
            kick_failure_from(&packets, 10, 20, 100).as_deref(),
            Some(
                "카카오 서버가 강퇴 요청을 거부했습니다 (status=-500): You can't remove a user from a chatroom."
            )
        );
        assert!(kick_failure_from(&packets, 10, 21, 100).is_none());
        assert!(kick_failure_from(&packets, 10, 20, 103).is_none());
    }

    fn loco_packet(direction: &str, packet_id: i32, body: &str, captured_at: i64) -> LocoPacket {
        LocoPacket {
            id: packet_id as u64,
            direction: direction.to_string(),
            method: "KICKMEM".to_string(),
            packet_id,
            status: 0,
            body_length: body.len() as i32,
            body: body.to_string(),
            captured_at,
        }
    }
}
