use super::state::NEXT_COMMAND_ID;
use crate::{
    intercept::{DatabaseInvalidation, record_database_invalidation, record_loco_packet},
    model::LocoPacket,
};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    net::{TcpListener, TcpStream},
    sync::atomic::Ordering,
    thread,
};
use tracing::{error, warn};
pub(super) fn launch_kakao_event_bridge(listener: TcpListener, token: String) {
    if let Err(error) = thread::Builder::new()
        .name("noa-kakao-loco".to_string())
        .spawn(move || {
            for incoming in listener.incoming() {
                match incoming {
                    Ok(stream) => {
                        let token = token.clone();
                        let _ = thread::Builder::new()
                            .name("noa-kakao-loco-stream".to_string())
                            .spawn(move || read_kakao_events(stream, &token));
                    }
                    Err(error) => warn!(%error, "KakaoTalk LOCO 이벤트 연결 수락 실패"),
                }
            }
        })
    {
        error!(%error, "KakaoTalk LOCO 이벤트 스레드를 시작하지 못했습니다");
    }
}

fn read_kakao_events(stream: TcpStream, token: &str) {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) => {
                warn!(%error, "KakaoTalk LOCO 이벤트 스트림 종료");
                return;
            }
        }
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if value.get("token").and_then(Value::as_str) != Some(token) {
            continue;
        }
        if value.get("event").and_then(Value::as_str) == Some("database-invalidated") {
            let Some(database) = value.get("database").and_then(Value::as_str) else {
                continue;
            };
            let Some(table) = value.get("table").and_then(Value::as_str) else {
                continue;
            };
            record_database_invalidation(DatabaseInvalidation {
                database: database.to_string(),
                table: table.to_string(),
                captured_at: value
                    .get("capturedAt")
                    .and_then(Value::as_i64)
                    .unwrap_or_default(),
            });
            continue;
        }
        if value.get("event").and_then(Value::as_str) != Some("loco") {
            continue;
        }
        record_loco_packet(LocoPacket {
            id: NEXT_COMMAND_ID.fetch_add(1, Ordering::Relaxed),
            direction: value
                .get("direction")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            method: value
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("NONE")
                .to_string(),
            packet_id: value
                .get("packetId")
                .and_then(Value::as_i64)
                .unwrap_or_default() as i32,
            status: value
                .get("status")
                .and_then(Value::as_i64)
                .unwrap_or_default() as i16,
            body_length: value
                .get("bodyLength")
                .and_then(Value::as_i64)
                .unwrap_or_default() as i32,
            body: value
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            captured_at: value
                .get("capturedAt")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
        });
    }
}
