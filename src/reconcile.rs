use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::sync::{RwLock, broadcast};
use tracing::{debug, warn};

use crate::{
    audit::AuditLog,
    failure::NoaError,
    kakao::RoomCatalog,
    model::{EventKind, FeedChange, Member, NewRoomEvent, Room, RoomEvent},
    settings::Settings,
};

type PresenceMap = HashMap<String, HashMap<String, Member>>;

pub fn launch(
    catalog: Arc<RoomCatalog>,
    audit: AuditLog,
    rooms_cache: Arc<RwLock<Vec<Room>>>,
    live_events: broadcast::Sender<RoomEvent>,
    config: Arc<Settings>,
) {
    tokio::spawn(async move {
        let mut cursor = query_task({
            let catalog = catalog.clone();
            move || catalog.feed_cursor()
        })
        .await
        .unwrap_or_default();

        let initial_rooms = query_task({
            let catalog = catalog.clone();
            move || catalog.snapshot()
        })
        .await
        .unwrap_or_default();
        let mut presence = index_members(&initial_rooms);
        *rooms_cache.write().await = initial_rooms;

        let mut invalidations = crate::intercept::subscribe_database_invalidations();
        let mut feed_safety = tokio::time::interval(Duration::from_millis(config.poll_interval_ms));
        let mut snapshot_safety =
            tokio::time::interval(Duration::from_millis(config.snapshot_interval_ms));
        feed_safety.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        snapshot_safety.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The initial state above already covers the immediate first interval ticks.
        feed_safety.tick().await;
        snapshot_safety.tick().await;

        loop {
            let mut refresh_feed = false;
            let mut refresh_snapshot = false;
            let room_triggered = tokio::select! {
                _ = feed_safety.tick() => {
                    refresh_feed = true;
                    false
                }
                _ = snapshot_safety.tick() => {
                    refresh_snapshot = true;
                    false
                }
                result = invalidations.recv() => {
                    apply_invalidation(result, &mut refresh_feed, &mut refresh_snapshot);
                    true
                }
            };

            if room_triggered {
                tokio::time::sleep(Duration::from_millis(50)).await;
                loop {
                    match invalidations.try_recv() {
                        Ok(invalidation) => apply_database_invalidation(
                            invalidation,
                            &mut refresh_feed,
                            &mut refresh_snapshot,
                        ),
                        Err(broadcast::error::TryRecvError::Lagged(_)) => {
                            refresh_feed = true;
                            refresh_snapshot = true;
                        }
                        Err(
                            broadcast::error::TryRecvError::Empty
                            | broadcast::error::TryRecvError::Closed,
                        ) => break,
                    }
                }
            }

            if refresh_feed {
                refresh_feed_changes(&catalog, &audit, &rooms_cache, &live_events, &mut cursor)
                    .await;
            }
            if refresh_snapshot {
                refresh_room_snapshot(&catalog, &audit, &rooms_cache, &live_events, &mut presence)
                    .await;
            }
        }
    });
}

fn apply_invalidation(
    result: Result<crate::intercept::DatabaseInvalidation, broadcast::error::RecvError>,
    refresh_feed: &mut bool,
    refresh_snapshot: &mut bool,
) {
    match result {
        Ok(invalidation) => {
            apply_database_invalidation(invalidation, refresh_feed, refresh_snapshot)
        }
        Err(broadcast::error::RecvError::Lagged(_)) => {
            *refresh_feed = true;
            *refresh_snapshot = true;
        }
        Err(broadcast::error::RecvError::Closed) => {}
    }
}

fn apply_database_invalidation(
    invalidation: crate::intercept::DatabaseInvalidation,
    refresh_feed: &mut bool,
    refresh_snapshot: &mut bool,
) {
    debug!(
        database = %invalidation.database,
        table = %invalidation.table,
        captured_at = invalidation.captured_at,
        "Room 데이터베이스 변경 감지"
    );
    match (invalidation.database.as_str(), invalidation.table.as_str()) {
        ("master", "chat_logs") => *refresh_feed = true,
        ("master", "chat_rooms")
        | ("secondary", "open_chat_member")
        | ("secondary", "open_link")
        | ("secondary", "open_profile") => *refresh_snapshot = true,
        _ => {}
    }
}

async fn refresh_feed_changes(
    catalog: &Arc<RoomCatalog>,
    audit: &AuditLog,
    rooms_cache: &RwLock<Vec<Room>>,
    live_events: &broadcast::Sender<RoomEvent>,
    cursor: &mut i64,
) {
    match query_task({
        let catalog = catalog.clone();
        let cursor = *cursor;
        move || catalog.changes_since(cursor, 250)
    })
    .await
    {
        Ok((last, changes)) => {
            *cursor = (*cursor).max(last);
            accept_feed(audit, rooms_cache, live_events, changes).await;
        }
        Err(error) => warn!(%error, "KakaoTalk 피드 증분 조회 실패"),
    }
}

async fn refresh_room_snapshot(
    catalog: &Arc<RoomCatalog>,
    audit: &AuditLog,
    rooms_cache: &RwLock<Vec<Room>>,
    live_events: &broadcast::Sender<RoomEvent>,
    presence: &mut PresenceMap,
) {
    let current_rooms = match query_task({
        let catalog = catalog.clone();
        move || catalog.snapshot()
    })
    .await
    {
        Ok(rooms) => rooms,
        Err(error) => {
            warn!(%error, "채팅방 참여자 스냅샷 갱신 실패");
            return;
        }
    };
    let current_presence = index_members(&current_rooms);
    for change in reconcile_presence(presence, &current_presence, &current_rooms) {
        persist_emit(audit, live_events, change);
    }
    *presence = current_presence;
    *rooms_cache.write().await = current_rooms;
}

async fn accept_feed(
    audit: &AuditLog,
    rooms_cache: &RwLock<Vec<Room>>,
    sender: &broadcast::Sender<RoomEvent>,
    changes: Vec<FeedChange>,
) {
    if changes.is_empty() {
        return;
    }
    let rooms = rooms_cache.read().await;
    for change in changes {
        let room_name = rooms
            .iter()
            .find(|room| room.chat_id == change.chat_id.to_string())
            .map(|room| room.name.clone())
            .unwrap_or_else(|| format!("채팅방 {}", change.chat_id));
        persist_emit(
            audit,
            sender,
            NewRoomEvent {
                chat_id: change.chat_id,
                room_name,
                kind: change.kind,
                user_id: change.user_id,
                nickname: change.nickname,
                previous_nickname: None,
                occurred_at: change.occurred_at,
                source: "feed",
                source_id: Some(change.database_id),
            },
        );
    }
}

fn persist_emit(audit: &AuditLog, sender: &broadcast::Sender<RoomEvent>, event: NewRoomEvent) {
    match audit.append(event) {
        Ok(Some(stored)) => {
            let _ = sender.send(stored);
        }
        Ok(None) => {}
        Err(error) => warn!(%error, "참여자 이벤트 저장 실패"),
    }
}

fn index_members(rooms: &[Room]) -> PresenceMap {
    rooms
        .iter()
        .map(|room| {
            (
                room.chat_id.clone(),
                room.members
                    .iter()
                    .cloned()
                    .map(|member| (member.user_id.clone(), member))
                    .collect(),
            )
        })
        .collect()
}

fn reconcile_presence(
    previous: &PresenceMap,
    current: &PresenceMap,
    rooms: &[Room],
) -> Vec<NewRoomEvent> {
    let mut events = Vec::new();
    let occurred_at = now();
    let room_names: HashMap<&str, &str> = rooms
        .iter()
        .map(|room| (room.chat_id.as_str(), room.name.as_str()))
        .collect();
    for (chat_id, current_members) in current {
        let Some(previous_members) = previous.get(chat_id) else {
            continue;
        };
        let Ok(numeric_chat_id) = chat_id.parse::<i64>() else {
            continue;
        };
        let room_name = room_names
            .get(chat_id.as_str())
            .copied()
            .unwrap_or(chat_id)
            .to_string();
        for (user_id, member) in current_members {
            let Ok(numeric_user_id) = user_id.parse::<i64>() else {
                continue;
            };
            match previous_members.get(user_id) {
                None => events.push(NewRoomEvent {
                    chat_id: numeric_chat_id,
                    room_name: room_name.clone(),
                    kind: EventKind::Joined,
                    user_id: numeric_user_id,
                    nickname: member.nickname.clone(),
                    previous_nickname: None,
                    occurred_at,
                    source: "snapshot",
                    source_id: None,
                }),
                Some(previous) if previous.nickname != member.nickname => {
                    events.push(NewRoomEvent {
                        chat_id: numeric_chat_id,
                        room_name: room_name.clone(),
                        kind: EventKind::NicknameChanged,
                        user_id: numeric_user_id,
                        nickname: member.nickname.clone(),
                        previous_nickname: Some(previous.nickname.clone()),
                        occurred_at,
                        source: "snapshot",
                        source_id: None,
                    })
                }
                _ => {}
            }
        }
        for (user_id, member) in previous_members {
            if current_members.contains_key(user_id) {
                continue;
            }
            let Ok(numeric_user_id) = user_id.parse::<i64>() else {
                continue;
            };
            events.push(NewRoomEvent {
                chat_id: numeric_chat_id,
                room_name: room_name.clone(),
                kind: EventKind::Left,
                user_id: numeric_user_id,
                nickname: member.nickname.clone(),
                previous_nickname: None,
                occurred_at,
                source: "snapshot",
                source_id: None,
            });
        }
    }
    debug!(count = events.len(), "참여자 스냅샷 비교 완료");
    events
}

async fn query_task<T>(
    operation: impl FnOnce() -> Result<T, NoaError> + Send + 'static,
) -> Result<T, NoaError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| NoaError::Internal(format!("DB 작업 조인 실패: {error}")))?
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, name: &str) -> Member {
        Member {
            user_id: id.to_string(),
            nickname: name.to_string(),
            profile_image_url: None,
            is_mine: false,
        }
    }

    #[test]
    fn detects_join_leave_and_nickname_change() {
        let previous: PresenceMap = [(
            "1".to_string(),
            [
                ("10".to_string(), member("10", "이전")),
                ("20".to_string(), member("20", "퇴장")),
            ]
            .into_iter()
            .collect(),
        )]
        .into_iter()
        .collect();
        let current_room = Room {
            chat_id: "1".to_string(),
            name: "방".to_string(),
            room_type: "M".to_string(),
            member_count: 2,
            members: vec![member("10", "변경"), member("30", "입장")],
        };
        let current = index_members(std::slice::from_ref(&current_room));
        let events = reconcile_presence(&previous, &current, &[current_room]);
        assert!(events.iter().any(|event| event.kind == EventKind::Joined));
        assert!(events.iter().any(|event| event.kind == EventKind::Left));
        assert!(
            events
                .iter()
                .any(|event| event.kind == EventKind::NicknameChanged)
        );
    }

    #[test]
    fn routes_room_invalidations_to_the_smallest_refresh() {
        let mut feed = false;
        let mut snapshot = false;
        apply_database_invalidation(
            crate::intercept::DatabaseInvalidation {
                database: "master".to_string(),
                table: "chat_logs".to_string(),
                captured_at: 1,
            },
            &mut feed,
            &mut snapshot,
        );
        assert!(feed);
        assert!(!snapshot);

        feed = false;
        apply_database_invalidation(
            crate::intercept::DatabaseInvalidation {
                database: "secondary".to_string(),
                table: "open_chat_member".to_string(),
                captured_at: 2,
            },
            &mut feed,
            &mut snapshot,
        );
        assert!(!feed);
        assert!(snapshot);
    }
}
