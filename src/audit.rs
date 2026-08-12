use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    failure::NoaError,
    model::{EventKind, NewRoomEvent, RoomEvent},
};

#[derive(Clone)]
pub struct AuditLog {
    connection: Arc<Mutex<Connection>>,
}

impl AuditLog {
    pub fn open_archive(path: &Path) -> Result<Self, NoaError> {
        let connection = Connection::open(path)
            .map_err(|error| NoaError::Database(format!("이벤트 DB 열기 실패: {error}")))?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS room_events (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               chat_id TEXT NOT NULL,
               room_name TEXT NOT NULL,
               kind TEXT NOT NULL,
               user_id TEXT NOT NULL,
               nickname TEXT NOT NULL,
               previous_nickname TEXT,
               occurred_at INTEGER NOT NULL,
               source TEXT NOT NULL,
               source_key TEXT UNIQUE
             );
             CREATE INDEX IF NOT EXISTS room_events_chat_time
               ON room_events(chat_id, occurred_at DESC);
             CREATE INDEX IF NOT EXISTS room_events_chat_user_time
               ON room_events(chat_id, user_id, occurred_at DESC);
             CREATE INDEX IF NOT EXISTS room_events_time
               ON room_events(occurred_at DESC);",
            )
            .map_err(|error| NoaError::Database(format!("이벤트 DB 초기화 실패: {error}")))?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn append(&self, event: NewRoomEvent) -> Result<Option<RoomEvent>, NoaError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| NoaError::Database("이벤트 DB 잠금 실패".to_string()))?;
        let recent = if event.kind == EventKind::NicknameChanged {
            connection.query_row(
                "SELECT id FROM room_events
                 WHERE chat_id = ?1 AND kind = ?2 AND user_id = ?3 AND nickname = ?4
                   AND occurred_at BETWEEN ?5 AND ?6 LIMIT 1",
                params![
                    event.chat_id.to_string(),
                    event.kind.as_str(),
                    event.user_id.to_string(),
                    event.nickname,
                    event.occurred_at - 3,
                    event.occurred_at + 3,
                ],
                |row| row.get::<_, i64>(0),
            )
        } else {
            connection.query_row(
                "SELECT id FROM room_events
                 WHERE chat_id = ?1 AND kind = ?2 AND user_id = ?3
                   AND occurred_at BETWEEN ?4 AND ?5 LIMIT 1",
                params![
                    event.chat_id.to_string(),
                    event.kind.as_str(),
                    event.user_id.to_string(),
                    event.occurred_at - 3,
                    event.occurred_at + 3,
                ],
                |row| row.get::<_, i64>(0),
            )
        }
        .optional()
        .map_err(|error| NoaError::Database(error.to_string()))?;
        if recent.is_some() {
            return Ok(None);
        }

        let source_key = event
            .source_id
            .map(|source_id| {
                format!(
                    "{}:{source_id}:{}:{}",
                    event.source,
                    event.kind.as_str(),
                    event.user_id
                )
            })
            .unwrap_or_else(|| {
                format!(
                    "{}:{}:{}:{}:{}:{}",
                    event.source,
                    event.chat_id,
                    event.kind.as_str(),
                    event.user_id,
                    event.occurred_at,
                    event.nickname
                )
            });
        let changed = connection
            .execute(
                "INSERT OR IGNORE INTO room_events
             (chat_id, room_name, kind, user_id, nickname, previous_nickname,
              occurred_at, source, source_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    event.chat_id.to_string(),
                    event.room_name,
                    event.kind.as_str(),
                    event.user_id.to_string(),
                    event.nickname,
                    event.previous_nickname,
                    event.occurred_at,
                    event.source,
                    source_key,
                ],
            )
            .map_err(|error| NoaError::Database(format!("이벤트 기록 실패: {error}")))?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(Some(RoomEvent {
            id: connection.last_insert_rowid(),
            chat_id: event.chat_id.to_string(),
            room_name: event.room_name,
            kind: event.kind,
            user_id: event.user_id.to_string(),
            nickname: event.nickname,
            previous_nickname: event.previous_nickname,
            occurred_at: event.occurred_at,
            source: event.source.to_string(),
        }))
    }

    pub fn recent(&self, chat_id: Option<i64>, limit: usize) -> Result<Vec<RoomEvent>, NoaError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| NoaError::Database("이벤트 DB 잠금 실패".to_string()))?;
        let limit = limit.clamp(1, 1_000) as i64;
        let (sql, id) = if let Some(chat_id) = chat_id {
            (
                "SELECT id, chat_id, room_name, kind, user_id, nickname,
                        previous_nickname, occurred_at, source
                 FROM room_events WHERE chat_id = ?1 ORDER BY occurred_at DESC, id DESC LIMIT ?2",
                Some(chat_id.to_string()),
            )
        } else {
            (
                "SELECT id, chat_id, room_name, kind, user_id, nickname,
                        previous_nickname, occurred_at, source
                 FROM room_events ORDER BY occurred_at DESC, id DESC LIMIT ?2",
                None,
            )
        };
        let mut statement = connection
            .prepare(sql)
            .map_err(|error| NoaError::Database(error.to_string()))?;
        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<RoomEvent> {
            let kind: String = row.get(3)?;
            Ok(RoomEvent {
                id: row.get(0)?,
                chat_id: row.get(1)?,
                room_name: row.get(2)?,
                kind: EventKind::from_db(&kind).unwrap_or(EventKind::NicknameChanged),
                user_id: row.get(4)?,
                nickname: row.get(5)?,
                previous_nickname: row.get(6)?,
                occurred_at: row.get(7)?,
                source: row.get(8)?,
            })
        };
        let rows = if let Some(id) = id {
            statement.query_map(params![id, limit], map_row)
        } else {
            statement.query_map(params![rusqlite::types::Null, limit], map_row)
        }
        .map_err(|error| NoaError::Database(error.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| NoaError::Database(error.to_string()))
    }

    pub fn recent_for_user(
        &self,
        chat_id: i64,
        user_id: i64,
        limit: usize,
    ) -> Result<Vec<RoomEvent>, NoaError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| NoaError::Database("이벤트 DB 잠금 실패".to_string()))?;
        let mut statement = connection
            .prepare(
                "SELECT id, chat_id, room_name, kind, user_id, nickname,
                        previous_nickname, occurred_at, source
                 FROM room_events
                 WHERE chat_id = ?1 AND user_id = ?2
                 ORDER BY occurred_at DESC, id DESC LIMIT ?3",
            )
            .map_err(|error| NoaError::Database(error.to_string()))?;
        let rows = statement
            .query_map(
                params![
                    chat_id.to_string(),
                    user_id.to_string(),
                    limit.clamp(1, 1_000) as i64
                ],
                |row| {
                    let kind: String = row.get(3)?;
                    Ok(RoomEvent {
                        id: row.get(0)?,
                        chat_id: row.get(1)?,
                        room_name: row.get(2)?,
                        kind: EventKind::from_db(&kind).unwrap_or(EventKind::NicknameChanged),
                        user_id: row.get(4)?,
                        nickname: row.get(5)?,
                        previous_nickname: row.get(6)?,
                        occurred_at: row.get(7)?,
                        source: row.get(8)?,
                    })
                },
            )
            .map_err(|error| NoaError::Database(error.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| NoaError::Database(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn persists_and_deduplicates_events() {
        let directory = tempdir().unwrap();
        let store = AuditLog::open_archive(&directory.path().join("events.db")).unwrap();
        let make = || NewRoomEvent {
            chat_id: 10,
            room_name: "테스트".to_string(),
            kind: EventKind::Joined,
            user_id: 20,
            nickname: "노아".to_string(),
            previous_nickname: None,
            occurred_at: 100,
            source: "feed",
            source_id: Some(1),
        };
        assert!(store.append(make()).unwrap().is_some());
        assert!(store.append(make()).unwrap().is_none());
        assert_eq!(store.recent(Some(10), 10).unwrap().len(), 1);
        assert_eq!(store.recent(None, 10).unwrap().len(), 1);
        assert_eq!(store.recent_for_user(10, 20, 10).unwrap().len(), 1);
        assert!(store.recent_for_user(10, 30, 10).unwrap().is_empty());
    }
}
