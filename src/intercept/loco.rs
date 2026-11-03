use crate::model::LocoPacket;

pub(super) fn kick_failure_detail(room_id: i64, user_id: i64, since: i64) -> Option<String> {
    kick_response(room_id, user_id, since).and_then(Result::err)
}

pub(super) fn kick_response(room_id: i64, user_id: i64, since: i64) -> Option<Result<(), String>> {
    kick_response_from(&super::loco_packets(64), room_id, user_id, since)
}

pub(super) fn unban_response(
    link_id: i64,
    room_id: i64,
    user_id: i64,
    since: i64,
) -> Option<Result<(), String>> {
    moderation_response_from(
        &super::loco_packets(64),
        "KLDELITEM",
        room_id,
        user_id,
        Some(link_id),
        since,
    )
}

fn kick_response_from(
    packets: &[LocoPacket],
    room_id: i64,
    user_id: i64,
    since: i64,
) -> Option<Result<(), String>> {
    moderation_response_from(packets, "KICKMEM", room_id, user_id, None, since)
}

fn moderation_response_from(
    packets: &[LocoPacket],
    method: &str,
    room_id: i64,
    user_id: i64,
    link_id: Option<i64>,
    since: i64,
) -> Option<Result<(), String>> {
    let (user_field, action) = match method {
        "KICKMEM" => ("mid", "강퇴"),
        "KLDELITEM" => ("kid", "강퇴 해제"),
        _ => return None,
    };
    let room = room_id.to_string();
    let user = user_id.to_string();
    let link = link_id.map(|id| id.to_string());
    let request = packets.iter().find(|packet| {
        packet.direction == "send"
            && packet.method == method
            && packet.captured_at >= since
            && body_field(&packet.body, "c") == Some(room.as_str())
            && body_field(&packet.body, user_field) == Some(user.as_str())
            && link
                .as_deref()
                .is_none_or(|link| body_field(&packet.body, "li") == Some(link))
    })?;
    let response = packets.iter().find(|packet| {
        packet.direction == "receive"
            && packet.method == method
            && packet.packet_id == request.packet_id
            && packet.captured_at >= request.captured_at
    })?;
    let body_status =
        body_field(&response.body, "status").and_then(|value| value.parse::<i32>().ok());
    let message = error_message(&response.body);
    if response.status == 0 && body_status.is_none_or(|status| status == 0) && message.is_none() {
        return Some(Ok(()));
    }
    let status = body_status
        .filter(|status| *status != 0)
        .map(|status| status.to_string())
        .unwrap_or_else(|| response.status.to_string());
    let reason = message.unwrap_or_else(|| response.body.trim().to_string());
    Some(Err(format!(
        "카카오 서버가 {action} 요청을 거부했습니다 (status={status}): {reason}"
    )))
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
    use super::{kick_response_from, moderation_response_from};
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
            kick_response_from(&packets, 10, 20, 100).unwrap(),
            Err(
                "카카오 서버가 강퇴 요청을 거부했습니다 (status=-500): You can't remove a user from a chatroom.".to_string()
            )
        );
        assert!(kick_response_from(&packets, 10, 21, 100).is_none());
        assert!(kick_response_from(&packets, 10, 20, 103).is_none());
    }

    #[test]
    fn kick_confirmation_requires_a_matching_server_response() {
        let request = loco_packet("send", 42, "{li=7, c=10, mid=20, r=false}", 101);
        assert_eq!(kick_response_from(&[], 10, 20, 100), None);
        assert_eq!(kick_response_from(&[request.clone()], 10, 20, 100), None);
        for mut response in [
            loco_packet("receive", 41, "{status=0}", 102),
            loco_packet("receive", 42, "{status=0}", 99),
            loco_packet("send", 42, "{status=0}", 102),
        ] {
            assert_eq!(
                kick_response_from(&[request.clone(), response.clone()], 10, 20, 100),
                None
            );
            response.direction = "receive".into();
            response.packet_id = 42;
            response.captured_at = 102;
            response.method = "KLDELITEM".into();
            assert_eq!(
                kick_response_from(&[request.clone(), response], 10, 20, 100),
                None
            );
        }
        let mut response = loco_packet("receive", 42, "{status=0}", 102);
        assert_eq!(
            kick_response_from(&[request.clone(), response.clone()], 10, 20, 100),
            Some(Ok(()))
        );
        response.status = -500;
        assert!(
            kick_response_from(&[request, response], 10, 20, 100)
                .unwrap()
                .unwrap_err()
                .contains("status=-500")
        );
    }

    #[test]
    fn unban_confirmation_matches_link_room_user_method_and_packet_id() {
        let packet = |direction, id, body, time| {
            let mut packet = loco_packet(direction, id, body, time);
            packet.method = "KLDELITEM".into();
            packet
        };
        let request = packet("send", 42, "{li=7, c=10, kid=9007199254740993}", 101);
        let response = packet("receive", 42, "{status=0}", 102);
        let target = 9007199254740993;
        let check = |packets: &[LocoPacket]| {
            moderation_response_from(packets, "KLDELITEM", 10, target, Some(7), 100)
        };
        assert_eq!(check(&[]), None);
        assert_eq!(check(&[request.clone()]), None);
        assert_eq!(check(&[response.clone()]), None);
        assert_eq!(check(&[request.clone(), response.clone()]), Some(Ok(())));
        for body in [
            "{li=8, c=10, kid=9007199254740993}",
            "{li=7, c=11, kid=9007199254740993}",
            "{li=7, c=10, kid=9007199254740992}",
            "{li=7, c=10, mid=9007199254740993}",
        ] {
            assert_eq!(
                check(&[packet("send", 42, body, 101), response.clone()]),
                None
            );
        }
        let mut wrong_response = response.clone();
        wrong_response.method = "KICKMEM".into();
        assert_eq!(check(&[request.clone(), wrong_response]), None);
        assert_eq!(
            check(&[request.clone(), packet("receive", 41, "{status=0}", 102)]),
            None
        );
        assert_eq!(
            check(&[request.clone(), packet("receive", 42, "{status=0}", 99)]),
            None
        );
        let mut stale_request = request.clone();
        stale_request.captured_at = 99;
        assert_eq!(check(&[stale_request, response]), None);
        let error = check(&[
            request,
            packet("receive", 42, "{status=-500, errMsg=unban denied}", 102),
        ])
        .unwrap()
        .unwrap_err();
        assert!(error.contains("강퇴 해제"));
        assert!(error.contains("status=-500"));
        assert!(error.contains("unban denied"));
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
