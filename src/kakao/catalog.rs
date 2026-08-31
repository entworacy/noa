use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;

use super::seal::{SealedText, profile_database_password};
use crate::{
    failure::NoaError,
    model::{EventKind, FeedChange, Member, Room},
    settings::Settings,
};

const OPEN_CHANNEL_ID_MASK: i64 = 1 << 54;

pub struct RoomCatalog {
    connection: Mutex<Connection>,
    primary_path: PathBuf,
    current_user_id: i64,
    cipher: SealedText,
    has_user_database: bool,
    has_profile_database: bool,
}

impl RoomCatalog {
    pub fn mount(config: &Settings) -> Result<Self, NoaError> {
        let app_path = config.kakao_path.as_ref().ok_or_else(|| {
            NoaError::Database(
                "KakaoTalk 앱 경로를 찾지 못했습니다. NOA_KAKAO_PATH를 지정하세요".to_string(),
            )
        })?;
        let database_dir = app_path.join("databases");
        let primary = database_dir.join("KakaoTalk.db");
        if !primary.exists() {
            return Err(NoaError::Database(format!(
                "KakaoTalk.db가 없습니다: {}",
                primary.display()
            )));
        }
        let connection = Connection::open_in_memory()
            .map_err(|error| NoaError::Database(format!("SQLite 초기화 실패: {error}")))?;
        attach(&connection, &primary, "db1", None)?;

        let secondary = database_dir.join("KakaoTalk2.db");
        if secondary.exists() {
            attach(&connection, &secondary, "db2", None)?;
        }

        let user_database = [
            database_dir.join("crypto_user_database"),
            database_dir.join("crypto_user_database.db"),
        ]
        .into_iter()
        .find(|path| path.exists());
        let preferences_path =
            app_path.join("files/datastore/Feature_DataStore.pref.preferences_pb");
        let has_user_database = match (user_database, fs::read(preferences_path)) {
            (Some(path), Ok(preferences)) => match profile_database_password(&preferences) {
                Ok(key) => attach(&connection, &path, "user", Some(&key)).is_ok(),
                Err(_) => false,
            },
            _ => false,
        };
        let profile_database = database_dir.join("multi_profile_database.db");
        let has_profile_database = profile_database.exists()
            && attach(&connection, &profile_database, "profile", None).is_ok();
        let current_user_id = infer_account_id(&connection).unwrap_or_default();
        connection
            .pragma_update(None, "query_only", true)
            .map_err(|error| {
                NoaError::Database(format!("KakaoTalk DB 읽기 전용 설정 실패: {error}"))
            })?;
        Ok(Self {
            connection: Mutex::new(connection),
            primary_path: primary,
            current_user_id,
            cipher: SealedText::for_owner(current_user_id),
            has_user_database,
            has_profile_database,
        })
    }

    pub fn current_user_id(&self) -> i64 {
        self.current_user_id
    }

    pub fn owned_profiles(&self) -> Result<Vec<OwnedProfile>, NoaError> {
        let connection = self.lock()?;
        let mut profiles = Vec::new();
        let mut profile_ids = HashSet::new();

        if self.has_profile_database && table_exists(&connection, "profile", "multi_profiles") {
            let columns = table_columns(&connection, "profile", "multi_profiles")?;
            let required = ["profileId", "nickName", "isMain", "order", "encryptType"];
            if required.iter().all(|column| columns.contains(*column)) {
                let original_image = if columns.contains("originalProfileImageURL") {
                    "originalProfileImageURL"
                } else {
                    "NULL"
                };
                let profile_image = if columns.contains("profileImageURL") {
                    "profileImageURL"
                } else {
                    "NULL"
                };
                let sql = format!(
                    "SELECT profileId, nickName, {original_image}, {profile_image},
                            isMain, \"order\", encryptType
                     FROM profile.multi_profiles
                     ORDER BY isMain DESC, \"order\" ASC, profileId ASC"
                );
                let mut statement = connection.prepare(&sql).map_err(|error| {
                    NoaError::Database(format!("소유 Kakao 프로필 조회 준비 실패: {error}"))
                })?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, bool>(4)?,
                            row.get::<_, i64>(5)?,
                            row.get::<_, u32>(6)?,
                        ))
                    })
                    .map_err(|error| {
                        NoaError::Database(format!("소유 Kakao 프로필 조회 실패: {error}"))
                    })?;
                for row in rows {
                    let (profile_id, nickname, original_image, profile_image, is_main, _, enc) =
                        row.map_err(|error| {
                            NoaError::Database(format!("소유 Kakao 프로필 해석 실패: {error}"))
                        })?;
                    let nickname = nickname
                        .map(|value| self.cipher.profile(&value, enc))
                        .unwrap_or_default();
                    if profile_id.trim().is_empty() || nickname.trim().is_empty() {
                        continue;
                    }
                    let profile_image_url = original_image
                        .or(profile_image)
                        .filter(|value| !value.is_empty())
                        .map(|value| self.cipher.profile(&value, enc));
                    profile_ids.insert(profile_id.clone());
                    profiles.push(OwnedProfile {
                        profile_id,
                        nickname,
                        profile_image_url,
                        kind: OwnedProfileKind::Kakao,
                        is_main,
                    });
                }
            }
        }

        if table_exists(&connection, "db2", "open_link") {
            let columns = table_columns(&connection, "db2", "open_link")?;
            if ["id", "name", "type"]
                .iter()
                .all(|column| columns.contains(*column))
            {
                let image = if columns.contains("image_url") {
                    "image_url"
                } else {
                    "NULL"
                };
                let active = if columns.contains("active") {
                    "AND COALESCE(active, 1) = 1"
                } else {
                    ""
                };
                let expired = if columns.contains("expired") {
                    "AND COALESCE(expired, 0) = 0"
                } else {
                    ""
                };
                let sql = format!(
                    "SELECT id, name, {image} FROM db2.open_link
                     WHERE type = 1 {active} {expired} ORDER BY id ASC"
                );
                let mut statement = connection.prepare(&sql).map_err(|error| {
                    NoaError::Database(format!("소유 오픈프로필 조회 준비 실패: {error}"))
                })?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    })
                    .map_err(|error| {
                        NoaError::Database(format!("소유 오픈프로필 조회 실패: {error}"))
                    })?;
                for row in rows {
                    let (profile_id, nickname, profile_image_url) = row.map_err(|error| {
                        NoaError::Database(format!("소유 오픈프로필 해석 실패: {error}"))
                    })?;
                    let profile_id = profile_id.to_string();
                    let nickname = nickname.unwrap_or_default();
                    if profile_id.parse::<i64>().is_err()
                        || nickname.trim().is_empty()
                        || !profile_ids.insert(profile_id.clone())
                    {
                        continue;
                    }
                    profiles.push(OwnedProfile {
                        profile_id,
                        nickname,
                        profile_image_url: profile_image_url.filter(|value| !value.is_empty()),
                        kind: OwnedProfileKind::OpenProfile,
                        is_main: false,
                    });
                }
            }
        }

        Ok(profiles)
    }

    pub fn open_profile_share_target(&self, link_id: i64) -> Result<Option<String>, NoaError> {
        if link_id <= 0 {
            return Err(NoaError::BadRequest(
                "linkId는 0보다 큰 정수여야 합니다".to_string(),
            ));
        }
        let connection = self.lock()?;
        if !table_exists(&connection, "db2", "open_link") {
            return Err(NoaError::Database(
                "KakaoTalk open_link 테이블을 찾지 못했습니다".to_string(),
            ));
        }
        let columns = table_columns(&connection, "db2", "open_link")?;
        if !["id", "url", "type"]
            .iter()
            .all(|column| columns.contains(*column))
        {
            return Err(NoaError::Database(
                "KakaoTalk open_link 테이블에 id, url 또는 type 열이 없습니다".to_string(),
            ));
        }
        let active = if columns.contains("active") {
            "COALESCE(active, 1)"
        } else {
            "1"
        };
        let expired = if columns.contains("expired") {
            "COALESCE(expired, 0)"
        } else {
            "0"
        };
        let sql = format!(
            "SELECT url, {active}, {expired}, type FROM db2.open_link WHERE id = ?1 LIMIT 1"
        );
        let row = connection
            .query_row(&sql, [link_id], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, bool>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .optional()
            .map_err(|error| {
                NoaError::Database(format!("오픈프로필 공유 링크 조회 실패: {error}"))
            })?;
        let Some(row) = row else {
            if table_exists(&connection, "db2", "open_chat_member") {
                let member_columns = table_columns(&connection, "db2", "open_chat_member")?;
                if member_columns.contains("profile_link_id") {
                    let is_member_profile = connection
                        .query_row(
                            "SELECT 1 FROM db2.open_chat_member
                             WHERE profile_link_id = ?1 AND profile_link_id > 0 LIMIT 1",
                            [link_id],
                            |_| Ok(()),
                        )
                        .optional()
                        .map_err(|error| {
                            NoaError::Database(format!(
                                "오픈채팅 멤버 프로필 linkId 조회 실패: {error}"
                            ))
                        })?
                        .is_some();
                    if is_member_profile {
                        return Ok(None);
                    }
                }
            }
            return Err(NoaError::NotFound(format!(
                "linkId에 해당하는 오픈프로필이 없습니다: {link_id}"
            )));
        };
        if !row.1 || row.2 {
            return Err(NoaError::NotFound(format!(
                "비활성화되었거나 만료된 오픈프로필입니다: {link_id}"
            )));
        }
        if row.3 != 1 {
            return Err(NoaError::NotFound(format!(
                "linkId가 오픈프로필을 가리키지 않습니다: {link_id}"
            )));
        }
        let url = row.0.unwrap_or_default().trim().to_string();
        if !is_open_link_url(&url) {
            return Err(NoaError::Database(format!(
                "오픈프로필 공유 링크가 올바르지 않습니다: {link_id}"
            )));
        }
        Ok(Some(url))
    }

    pub fn member_open_profile_link_id(
        &self,
        chat_id: i64,
        user_id: i64,
    ) -> Result<Option<i64>, NoaError> {
        if chat_id <= 0 || user_id <= 0 {
            return Err(NoaError::BadRequest(
                "chatId와 userId는 0보다 큰 정수여야 합니다".to_string(),
            ));
        }
        let connection = self.lock()?;
        if !table_exists(&connection, "db2", "open_chat_member") {
            return Err(NoaError::Database(
                "KakaoTalk open_chat_member 테이블을 찾지 못했습니다".to_string(),
            ));
        }
        let columns = table_columns(&connection, "db2", "open_chat_member")?;
        if !columns.contains("user_id") || !columns.contains("profile_link_id") {
            return Err(NoaError::Database(
                "KakaoTalk open_chat_member 테이블에 user_id 또는 profile_link_id 열이 없습니다"
                    .to_string(),
            ));
        }
        let room_predicate = if columns.contains("involved_chat_id") {
            "involved_chat_id = ?1"
        } else if columns.contains("link_id") {
            "link_id = (SELECT link_id FROM db1.chat_rooms WHERE id = ?1)"
        } else {
            return Err(NoaError::Database(
                "KakaoTalk open_chat_member 테이블에 방 식별 열이 없습니다".to_string(),
            ));
        };
        let sql = format!(
            "SELECT DISTINCT profile_link_id FROM db2.open_chat_member
             WHERE {room_predicate} AND user_id = ?2 AND profile_link_id > 0
             ORDER BY profile_link_id ASC LIMIT 2"
        );
        let mut statement = connection.prepare(&sql).map_err(|error| {
            NoaError::Database(format!("멤버 오픈프로필 linkId 조회 준비 실패: {error}"))
        })?;
        let link_ids = statement
            .query_map(params![chat_id, user_id], |row| row.get::<_, i64>(0))
            .map_err(|error| {
                NoaError::Database(format!("멤버 오픈프로필 linkId 조회 실패: {error}"))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                NoaError::Database(format!("멤버 오픈프로필 linkId 해석 실패: {error}"))
            })?;
        match link_ids.as_slice() {
            [] => Ok(None),
            [link_id] => Ok(Some(*link_id)),
            _ => Err(NoaError::Database(format!(
                "채팅방 {chat_id}의 userId {user_id}에 서로 다른 오픈프로필 linkId가 있습니다"
            ))),
        }
    }

    pub fn enqueue_custom(&self, draft: CustomMessageDraft) -> Result<QueuedMessage, NoaError> {
        let mut connection = Connection::open(&self.primary_path)
            .map_err(|error| NoaError::Database(format!("KakaoTalk DB 쓰기 연결 실패: {error}")))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| NoaError::Database(format!("KakaoTalk DB 대기 설정 실패: {error}")))?;
        let exists = connection
            .query_row(
                "SELECT 1 FROM chat_rooms WHERE id = ?1 LIMIT 1",
                [draft.chat_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| NoaError::Database(format!("채팅방 확인 실패: {error}")))?
            .is_some();
        if !exists {
            return Err(NoaError::NotFound(format!(
                "채팅방을 찾을 수 없습니다: {}",
                draft.chat_id
            )));
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let client_message_id = match draft.client_message_id {
            Some(value) => {
                if value <= 0 || client_message_id_exists(&connection, value)? {
                    return Err(NoaError::BadRequest(
                        "client_message_id는 양수이며 기존 값과 중복되지 않아야 합니다".to_string(),
                    ));
                }
                value
            }
            None => next_client_message_id(&connection, now_ms)?,
        };
        let created_at = draft.created_at.unwrap_or(now_ms / 1_000);
        let metadata = draft.metadata.unwrap_or_else(|| {
            serde_json::json!({
                "tempId": now_ms,
                "origin": "com.kakao.talk.notification.NotificationActionService",
                "fromInputBox": false
            })
            .to_string()
        });
        let transaction = connection
            .transaction()
            .map_err(|error| NoaError::Database(format!("발신 행 트랜잭션 실패: {error}")))?;
        transaction
            .execute(
                "INSERT INTO chat_sending_logs
                 (type, chat_id, thread_id, scope, message, attachment, created_at,
                  client_message_id, supplement, v, is_silence)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    draft.message_type,
                    draft.chat_id,
                    draft.thread_id,
                    draft.scope,
                    draft.message,
                    draft.attachment,
                    created_at,
                    client_message_id,
                    draft.supplement,
                    metadata,
                    draft.is_silence
                ],
            )
            .map_err(|error| NoaError::Database(format!("발신 행 등록 실패: {error}")))?;
        let row_id = transaction.last_insert_rowid();
        transaction
            .commit()
            .map_err(|error| NoaError::Database(format!("발신 행 저장 실패: {error}")))?;
        Ok(QueuedMessage {
            row_id,
            client_message_id,
        })
    }

    pub fn delivery_state(&self, client_message_id: i64) -> Result<DeliveryState, NoaError> {
        let connection = self.lock()?;
        let sending = connection
            .query_row(
                "SELECT 1 FROM db1.chat_sending_logs WHERE client_message_id = ?1 LIMIT 1",
                [client_message_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| NoaError::Database(format!("발신 대기 상태 조회 실패: {error}")))?
            .is_some();
        let delivered = connection
            .query_row(
                "SELECT 1 FROM db1.chat_logs WHERE client_message_id = ?1 LIMIT 1",
                [client_message_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| NoaError::Database(format!("발신 완료 상태 조회 실패: {error}")))?
            .is_some();
        Ok(match (sending, delivered) {
            (_, true) => DeliveryState::Delivered,
            (true, false) => DeliveryState::Waiting,
            (false, false) => DeliveryState::Missing,
        })
    }

    pub fn snapshot(&self) -> Result<Vec<Room>, NoaError> {
        let connection = self.lock()?;
        let columns = table_columns(&connection, "db1", "chat_rooms")?;
        let select = |name: &str| {
            if columns.contains(name) {
                name.to_string()
            } else {
                "NULL".to_string()
            }
        };
        let order = if columns.contains("last_log_id") {
            "last_log_id DESC"
        } else if columns.contains("last_updated_at") {
            "last_updated_at DESC"
        } else {
            "id DESC"
        };
        let sql = format!(
            "SELECT id, {}, {}, {}, {} FROM db1.chat_rooms ORDER BY {order}",
            select("type"),
            select("active_member_ids"),
            select("private_meta"),
            select("v"),
        );
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| NoaError::Database(format!("채팅방 조회 준비 실패: {error}")))?;
        let rows = statement
            .query_map([], |row| {
                Ok(RawRoom {
                    chat_id: row.get(0)?,
                    room_type: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    active_member_ids: row.get(2)?,
                    private_meta: row.get(3)?,
                    metadata: row.get(4)?,
                })
            })
            .map_err(|error| NoaError::Database(format!("채팅방 조회 실패: {error}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| NoaError::Database(format!("채팅방 행 해석 실패: {error}")))?;

        rows.into_iter()
            .map(|raw| self.map_room(&connection, raw))
            .collect()
    }

    pub fn room_snapshot(&self, chat_id: i64) -> Result<Room, NoaError> {
        let connection = self.lock()?;
        let columns = table_columns(&connection, "db1", "chat_rooms")?;
        let select = |name: &str| {
            if columns.contains(name) {
                name.to_string()
            } else {
                "NULL".to_string()
            }
        };
        let sql = format!(
            "SELECT id, {}, {}, {}, {} FROM db1.chat_rooms WHERE id = ?1 LIMIT 1",
            select("type"),
            select("active_member_ids"),
            select("private_meta"),
            select("v")
        );
        let raw = connection
            .query_row(&sql, [chat_id], |row| {
                Ok(RawRoom {
                    chat_id: row.get(0)?,
                    room_type: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    active_member_ids: row.get(2)?,
                    private_meta: row.get(3)?,
                    metadata: row.get(4)?,
                })
            })
            .optional()
            .map_err(|error| NoaError::Database(format!("채팅방 조회 실패: {error}")))?
            .ok_or_else(|| NoaError::NotFound(format!("채팅방을 찾을 수 없습니다: {chat_id}")))?;
        self.map_room(&connection, raw)
    }

    /// Loads the exact server-side log metadata required by KakaoTalk's
    /// open-chat message hiding operation.
    pub fn hide_message_target(
        &self,
        chat_id: i64,
        log_id: i64,
    ) -> Result<HideMessageTarget, NoaError> {
        if chat_id <= 0 || log_id <= 0 {
            return Err(NoaError::BadRequest(
                "chatId와 logId는 양수여야 합니다".to_string(),
            ));
        }
        let connection = self.lock()?;
        if !table_exists(&connection, "db1", "chat_logs") {
            return Err(NoaError::Database(
                "KakaoTalk chat_logs 테이블이 없습니다".to_string(),
            ));
        }
        let columns = table_columns(&connection, "db1", "chat_logs")?;
        for required in ["id", "type", "chat_id", "message"] {
            if !columns.contains(required) {
                return Err(NoaError::Database(format!(
                    "KakaoTalk chat_logs.{required} 열이 없습니다"
                )));
            }
        }
        connection
            .query_row(
                "SELECT type, message FROM db1.chat_logs
                 WHERE chat_id = ?1 AND id = ?2 LIMIT 1",
                params![chat_id, log_id],
                |row| {
                    Ok(HideMessageTarget {
                        message_type: row.get(0)?,
                        message: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    })
                },
            )
            .optional()
            .map_err(|error| NoaError::Database(format!("가리기 대상 메시지 조회 실패: {error}")))?
            .ok_or_else(|| {
                NoaError::NotFound(format!(
                    "채팅방 {chat_id}에서 메시지를 찾을 수 없습니다: {log_id}"
                ))
            })
    }

    /// Resolves the current open-chat voice-room endpoint from KakaoTalk's own
    /// chat log. The HTTP client never supplies VOX server coordinates.
    pub fn voiceroom_join_info(&self, chat_id: i64) -> Result<VoiceroomJoinInfo, NoaError> {
        let connection = self.lock()?;
        if !table_exists(&connection, "db1", "chat_logs") {
            return Err(NoaError::Database(
                "KakaoTalk chat_logs 테이블이 없습니다".to_string(),
            ));
        }
        let columns = table_columns(&connection, "db1", "chat_logs")?;
        for required in ["_id", "type", "chat_id", "attachment"] {
            if !columns.contains(required) {
                return Err(NoaError::Database(format!(
                    "KakaoTalk chat_logs.{required} 열이 없습니다"
                )));
            }
        }

        let mut statement = connection
            .prepare(
                "SELECT type, attachment
                 FROM db1.chat_logs
                 WHERE chat_id = ?1
                 ORDER BY _id DESC
                 LIMIT 100",
            )
            .map_err(|error| {
                NoaError::Database(format!("보이스룸 접속정보 조회 준비 실패: {error}"))
            })?;
        let rows = statement
            .query_map([chat_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            })
            .map_err(|error| NoaError::Database(format!("보이스룸 접속정보 조회 실패: {error}")))?;

        for row in rows {
            let (message_type, attachment) = row.map_err(|error| {
                NoaError::Database(format!("보이스룸 접속정보 행 해석 실패: {error}"))
            })?;
            if normalized_chat_message_type(message_type) != 52 {
                continue;
            }
            match parse_voiceroom_attachment(chat_id, &attachment)? {
                VoiceroomAttachment::Invite(info) => return Ok(info),
                VoiceroomAttachment::Ended => {
                    return Err(NoaError::NotFound(format!(
                        "진행 중인 오픈채팅 보이스톡이 없습니다: {chat_id}"
                    )));
                }
                VoiceroomAttachment::Other => {}
            }
        }
        Err(NoaError::NotFound(format!(
            "진행 중인 오픈채팅 보이스톡을 찾을 수 없습니다: {chat_id}"
        )))
    }

    /// Reads the authoritative active-member list for one room without rebuilding the
    /// complete room catalog. This is intentionally stricter than `snapshot`: an
    /// unreadable member list must not be mistaken for a successful removal.
    pub fn room_has_member(&self, chat_id: i64, user_id: i64) -> Result<bool, NoaError> {
        let connection = self.lock()?;
        let columns = table_columns(&connection, "db1", "chat_rooms")?;
        let select = |name: &str| {
            if columns.contains(name) {
                name.to_string()
            } else {
                "NULL".to_string()
            }
        };
        let sql = format!(
            "SELECT {}, {} FROM db1.chat_rooms WHERE id = ?1 LIMIT 1",
            select("active_member_ids"),
            select("v")
        );
        let row = connection
            .query_row(&sql, [chat_id], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })
            .optional()
            .map_err(|error| NoaError::Database(format!("채팅방 참여자 검증 조회 실패: {error}")))?
            .ok_or_else(|| NoaError::NotFound(format!("채팅방을 찾을 수 없습니다: {chat_id}")))?;
        let member_ids = member_ids(row.0.as_deref(), row.1.as_deref()).ok_or_else(|| {
            NoaError::Database(format!(
                "채팅방 {chat_id}의 활성 참여자 목록을 해석할 수 없습니다"
            ))
        })?;
        Ok(member_ids.contains(&user_id))
    }

    pub fn feed_cursor(&self) -> Result<i64, NoaError> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT COALESCE(MAX(_id), 0) FROM db1.chat_logs",
                [],
                |row| row.get(0),
            )
            .map_err(|error| NoaError::Database(format!("최신 메시지 ID 조회 실패: {error}")))
    }

    pub fn changes_since(
        &self,
        after: i64,
        limit: usize,
    ) -> Result<(i64, Vec<FeedChange>), NoaError> {
        let connection = self.lock()?;
        let columns = table_columns(&connection, "db1", "chat_logs")?;
        let created = if columns.contains("created_at") {
            "created_at"
        } else {
            "NULL"
        };
        let sql = format!(
            "SELECT _id, chat_id, user_id, message, v, {created}
             FROM db1.chat_logs WHERE _id > ?1 AND type = 0 ORDER BY _id ASC LIMIT ?2"
        );
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| NoaError::Database(format!("피드 조회 준비 실패: {error}")))?;
        let rows = statement
            .query_map(params![after, limit.clamp(1, 1_000) as i64], |row| {
                Ok(RawFeed {
                    database_id: row.get(0)?,
                    chat_id: row.get(1)?,
                    user_id: row.get(2)?,
                    message: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    metadata: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|error| NoaError::Database(format!("피드 조회 실패: {error}")))?;

        let mut last = after;
        let mut changes = Vec::new();
        for raw in rows {
            let raw = raw.map_err(|error| NoaError::Database(error.to_string()))?;
            last = last.max(raw.database_id);
            changes.extend(self.parse_feed(raw));
        }
        Ok((last, changes))
    }

    fn map_room(&self, connection: &Connection, raw: RawRoom) -> Result<Room, NoaError> {
        let member_ids = member_ids(raw.active_member_ids.as_deref(), raw.metadata.as_deref())
            .unwrap_or_default();
        let mut members = Vec::with_capacity(member_ids.len());
        for user_id in member_ids {
            members.push(self.member(connection, raw.chat_id, user_id));
        }
        let name = private_name(raw.private_meta.as_deref())
            .or_else(|| self.open_room_name(connection, raw.chat_id))
            .or_else(|| {
                let names: Vec<&str> = members
                    .iter()
                    .filter(|member| !member.is_mine)
                    .take(4)
                    .map(|member| member.nickname.as_str())
                    .collect();
                (!names.is_empty()).then(|| names.join(", "))
            })
            .unwrap_or_else(|| format!("채팅방 {}", raw.chat_id));
        Ok(Room {
            chat_id: raw.chat_id.to_string(),
            name,
            room_type: raw.room_type,
            member_count: members.len(),
            members,
        })
    }

    fn member(&self, connection: &Connection, chat_id: i64, user_id: i64) -> Member {
        if user_id == self.current_user_id {
            return self
                .regular_member(connection, user_id, true)
                .unwrap_or_else(|| fallback_member(user_id, true, "나"));
        }
        if chat_id & OPEN_CHANNEL_ID_MASK != 0
            && let Some(member) = self.open_member(connection, chat_id, user_id)
        {
            return member;
        }
        self.regular_member(connection, user_id, false)
            .or_else(|| self.friend_member(connection, user_id))
            .unwrap_or_else(|| fallback_member(user_id, false, &format!("사용자 {user_id}")))
    }

    fn regular_member(
        &self,
        connection: &Connection,
        user_id: i64,
        is_mine: bool,
    ) -> Option<Member> {
        if !self.has_user_database || !table_exists(connection, "user", "user") {
            return None;
        }
        connection
            .query_row(
                "SELECT nickname, original_profile_image_url FROM user.user WHERE id = ?1 LIMIT 1",
                [user_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .ok()
            .flatten()
            .map(|(nickname, profile_image_url)| Member {
                user_id: user_id.to_string(),
                nickname,
                profile_image_url,
                is_mine,
            })
    }

    fn friend_member(&self, connection: &Connection, user_id: i64) -> Option<Member> {
        if !table_exists(connection, "db2", "friends") {
            return None;
        }
        connection.query_row(
            "SELECT name, original_profile_image_url, enc FROM db2.friends WHERE id = ?1 LIMIT 1",
            [user_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, u32>(2)?)),
        ).optional().ok().flatten().map(|(nickname, profile, enc)| Member {
            user_id: user_id.to_string(),
            nickname: self.cipher.profile(&nickname, enc),
            profile_image_url: profile.map(|value| self.cipher.profile(&value, enc)),
            is_mine: false,
        })
    }

    fn open_member(&self, connection: &Connection, chat_id: i64, user_id: i64) -> Option<Member> {
        if !table_exists(connection, "db2", "open_chat_member") {
            return None;
        }
        connection
            .query_row(
                "SELECT nickname, original_profile_image_url, enc
             FROM db2.open_chat_member
             WHERE link_id = (SELECT link_id FROM db1.chat_rooms WHERE id = ?1)
               AND user_id = ?2 LIMIT 1",
                params![chat_id, user_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, u32>(2)?,
                    ))
                },
            )
            .optional()
            .ok()
            .flatten()
            .map(|(nickname, profile, enc)| Member {
                user_id: user_id.to_string(),
                nickname: self.cipher.profile(&nickname, enc),
                profile_image_url: profile.map(|value| self.cipher.profile(&value, enc)),
                is_mine: false,
            })
    }

    fn open_room_name(&self, connection: &Connection, chat_id: i64) -> Option<String> {
        if !table_exists(connection, "db2", "open_link") {
            return None;
        }
        connection
            .query_row(
                "SELECT name FROM db2.open_link
             WHERE id = (SELECT link_id FROM db1.chat_rooms WHERE id = ?1) LIMIT 1",
                [chat_id],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    fn parse_feed(&self, raw: RawFeed) -> Vec<FeedChange> {
        let enc = raw
            .metadata
            .as_deref()
            .and_then(|value| serde_json::from_str::<Value>(value).ok())
            .and_then(|value| value.get("enc").and_then(Value::as_u64))
            .unwrap_or_default() as u32;
        let message = self.cipher.reveal(&raw.message, enc, raw.user_id);
        let Ok(json) = serde_json::from_str::<Value>(&message) else {
            return Vec::new();
        };
        let feed_type = json
            .get("feedType")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let (kind, users): (EventKind, Vec<&Value>) = match feed_type {
            1 | 4 => (
                EventKind::Joined,
                json.get("members")
                    .and_then(Value::as_array)
                    .map(|v| v.iter().collect())
                    .unwrap_or_default(),
            ),
            2 => (EventKind::Left, json.get("member").into_iter().collect()),
            6 => (EventKind::Kicked, json.get("member").into_iter().collect()),
            _ => return Vec::new(),
        };
        let occurred_at = normalize_timestamp(raw.created_at);
        users
            .into_iter()
            .filter_map(|user| {
                let user_id = user.get("userId").and_then(json_i64)?;
                let nickname = user
                    .get("nickName")
                    .and_then(Value::as_str)
                    .unwrap_or("알 수 없음")
                    .to_string();
                Some(FeedChange {
                    database_id: raw.database_id,
                    chat_id: raw.chat_id,
                    kind: kind.clone(),
                    user_id,
                    nickname,
                    occurred_at,
                })
            })
            .collect()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, NoaError> {
        self.connection
            .lock()
            .map_err(|_| NoaError::Database("KakaoTalk DB 잠금 실패".to_string()))
    }
}

pub struct CustomMessageDraft {
    pub message_type: i64,
    pub chat_id: i64,
    pub thread_id: Option<i64>,
    pub scope: i64,
    pub message: String,
    pub attachment: String,
    pub created_at: Option<i64>,
    pub client_message_id: Option<i64>,
    pub supplement: Option<String>,
    pub metadata: Option<String>,
    pub is_silence: i64,
}

pub struct QueuedMessage {
    pub row_id: i64,
    pub client_message_id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HideMessageTarget {
    pub message_type: i64,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceroomJoinInfo {
    pub chat_id: i64,
    pub call_id: i64,
    pub host_v4: String,
    pub host_v6: String,
    pub port: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedProfile {
    pub profile_id: String,
    pub nickname: String,
    pub profile_image_url: Option<String>,
    pub kind: OwnedProfileKind,
    pub is_main: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OwnedProfileKind {
    Kakao,
    OpenProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryState {
    Waiting,
    Delivered,
    Missing,
}

struct RawRoom {
    chat_id: i64,
    room_type: String,
    active_member_ids: Option<String>,
    private_meta: Option<String>,
    metadata: Option<String>,
}

struct RawFeed {
    database_id: i64,
    chat_id: i64,
    user_id: i64,
    message: String,
    metadata: Option<String>,
    created_at: Option<i64>,
}

enum VoiceroomAttachment {
    Invite(VoiceroomJoinInfo),
    Ended,
    Other,
}

fn normalized_chat_message_type(value: i64) -> i64 {
    value & !16_384_i64 & !268_435_456_i64
}

fn parse_voiceroom_attachment(
    chat_id: i64,
    attachment: &str,
) -> Result<VoiceroomAttachment, NoaError> {
    let Ok(value) = serde_json::from_str::<Value>(attachment) else {
        return Ok(VoiceroomAttachment::Other);
    };
    match value.get("type").and_then(Value::as_str) {
        Some("vr_bye") => Ok(VoiceroomAttachment::Ended),
        Some("vr_invite") => {
            let call_id = optional_json_i64(value.get("callId"))
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    NoaError::Database("보이스룸 초대에 올바른 callId가 없습니다".to_string())
                })?;
            let host_v4 = value
                .get("csIP")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let host_v6 = value
                .get("csIP6")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            if (host_v4.is_empty() && host_v6.is_empty())
                || !valid_vox_host(&host_v4)
                || !valid_vox_host(&host_v6)
            {
                return Err(NoaError::Database(
                    "보이스룸 초대의 VOX 호스트가 올바르지 않습니다".to_string(),
                ));
            }
            let port = optional_json_i64(value.get("csPort"))
                .and_then(|value| i32::try_from(value).ok())
                .filter(|value| (1..=65_535).contains(value))
                .ok_or_else(|| {
                    NoaError::Database("보이스룸 초대의 VOX 포트가 올바르지 않습니다".to_string())
                })?;
            Ok(VoiceroomAttachment::Invite(VoiceroomJoinInfo {
                chat_id,
                call_id,
                host_v4,
                host_v6,
                port,
            }))
        }
        _ => Ok(VoiceroomAttachment::Other),
    }
}

fn optional_json_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str()?.parse::<i64>().ok())
    })
}

fn valid_vox_host(value: &str) -> bool {
    value.len() <= 255 && !value.chars().any(char::is_control)
}

fn attach(
    connection: &Connection,
    path: &Path,
    alias: &str,
    key: Option<&str>,
) -> Result<(), NoaError> {
    let path = path.to_string_lossy().replace("'", "''");
    let sql = if let Some(key) = key {
        format!("ATTACH DATABASE '{path}' AS {alias} KEY x'{key}'")
    } else {
        format!("ATTACH DATABASE '{path}' AS {alias}")
    };
    connection
        .execute_batch(&sql)
        .map_err(|error| NoaError::Database(format!("{} 연결 실패: {error}", path)))
}

fn infer_account_id(connection: &Connection) -> Option<i64> {
    connection
        .query_row(
            "SELECT user_id FROM db1.chat_logs
         WHERE v LIKE '%isMine\":true%' ORDER BY _id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
}

fn client_message_id_exists(connection: &Connection, value: i64) -> Result<bool, NoaError> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM chat_sending_logs WHERE client_message_id = ?1
                UNION ALL
                SELECT 1 FROM chat_logs WHERE client_message_id = ?1
            )",
            [value],
            |row| row.get(0),
        )
        .map_err(|error| NoaError::Database(format!("client_message_id 확인 실패: {error}")))
}

fn next_client_message_id(connection: &Connection, now_ms: i64) -> Result<i64, NoaError> {
    let mut value = (now_ms.rem_euclid(i32::MAX as i64) / 100) * 100 + 38;
    if value > i32::MAX as i64 {
        value = 38;
    }
    for _ in 0..10_000 {
        if !client_message_id_exists(connection, value)? {
            return Ok(value);
        }
        value += 100;
        if value > i32::MAX as i64 {
            value = 38;
        }
    }
    Err(NoaError::Database(
        "사용 가능한 client_message_id를 만들지 못했습니다".to_string(),
    ))
}

fn table_exists(connection: &Connection, database: &str, table: &str) -> bool {
    connection
        .query_row(
            &format!("SELECT 1 FROM {database}.sqlite_master WHERE type = 'table' AND name = ?1"),
            [table],
            |_| Ok(()),
        )
        .optional()
        .ok()
        .flatten()
        .is_some()
}

fn table_columns(
    connection: &Connection,
    database: &str,
    table: &str,
) -> Result<HashSet<String>, NoaError> {
    let mut statement = connection
        .prepare(&format!("PRAGMA {database}.table_info({table})"))
        .map_err(|error| NoaError::Database(format!("{table} 스키마 조회 실패: {error}")))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| NoaError::Database(error.to_string()))?;
    rows.collect::<Result<HashSet<_>, _>>()
        .map_err(|error| NoaError::Database(error.to_string()))
}

fn parse_id_array(raw: &str) -> Option<Vec<i64>> {
    let value: Value = serde_json::from_str(raw).ok()?;
    Some(value.as_array()?.iter().filter_map(json_i64).collect())
}

fn member_ids(active_member_ids: Option<&str>, metadata: Option<&str>) -> Option<Vec<i64>> {
    active_member_ids
        .and_then(parse_id_array)
        .or_else(|| metadata.and_then(display_ids))
}

fn display_ids(raw: &str) -> Option<Vec<i64>> {
    let value: Value = serde_json::from_str(raw).ok()?;
    value
        .get("displayUserIds")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(json_i64).collect())
}

fn json_i64(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}

fn is_open_link_url(value: &str) -> bool {
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    let Some(mut segments) = parsed.path_segments() else {
        return false;
    };
    let section = segments.next();
    let token = segments.next();
    parsed.scheme() == "https"
        && parsed.host_str() == Some("open.kakao.com")
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && matches!(section, Some("o" | "me"))
        && token.is_some_and(|value| {
            !value.is_empty()
                && value.bytes().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, b'_' | b'-')
                })
        })
        && segments.next().is_none()
}

fn private_name(raw: Option<&str>) -> Option<String> {
    serde_json::from_str::<Value>(raw?)
        .ok()?
        .get("name")?
        .as_str()
        .map(str::to_string)
        .filter(|name| !name.is_empty())
}

fn fallback_member(user_id: i64, is_mine: bool, nickname: &str) -> Member {
    Member {
        user_id: user_id.to_string(),
        nickname: nickname.to_string(),
        profile_image_url: None,
        is_mine,
    }
}

fn normalize_timestamp(value: Option<i64>) -> i64 {
    let value = value.unwrap_or_else(now);
    if value > 10_000_000_000 {
        value / 1_000
    } else {
        value
    }
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
    use tempfile::tempdir;

    #[test]
    fn parses_numeric_and_string_member_ids() {
        assert_eq!(parse_id_array(r#"[1,"2",null]"#), Some(vec![1, 2]));
        assert_eq!(
            display_ids(r#"{"displayUserIds":["3",4]}"#),
            Some(vec![3, 4])
        );
        assert_eq!(member_ids(Some("[1,2]"), None), Some(vec![1, 2]));
        assert_eq!(
            member_ids(None, Some(r#"{"displayUserIds":["3",4]}"#)),
            Some(vec![3, 4])
        );
        assert_eq!(member_ids(Some("invalid"), Some("{}")), None);
    }

    #[test]
    fn reads_private_room_name() {
        assert_eq!(
            private_name(Some(r#"{"name":"우리 방"}"#)).as_deref(),
            Some("우리 방")
        );
    }

    #[test]
    fn opens_fixture_schema_and_maps_room_and_feed() {
        let directory = tempdir().unwrap();
        let app_path = directory.path().join("com.kakao.talk");
        let databases = app_path.join("databases");
        std::fs::create_dir_all(&databases).unwrap();
        {
            let primary = Connection::open(databases.join("KakaoTalk.db")).unwrap();
            primary
                .execute_batch(
                    r#"
                CREATE TABLE chat_rooms (
                  id INTEGER PRIMARY KEY, type TEXT, active_member_ids TEXT,
                  private_meta TEXT, v TEXT, last_log_id INTEGER
                );
                CREATE TABLE chat_logs (
                  _id INTEGER PRIMARY KEY, id INTEGER, type INTEGER, chat_id INTEGER,
                  user_id INTEGER, message TEXT, attachment TEXT, v TEXT, created_at INTEGER,
                  client_message_id INTEGER
                );
                CREATE TABLE chat_sending_logs (
                  _id INTEGER PRIMARY KEY AUTOINCREMENT, type INTEGER, chat_id INTEGER NOT NULL,
                  thread_id INTEGER, scope INTEGER, message TEXT, attachment TEXT,
                  created_at INTEGER, client_message_id INTEGER, supplement TEXT, v TEXT,
                  is_silence INTEGER NOT NULL
                );
                INSERT INTO chat_rooms VALUES
                  (900, 'M', '[100,200]', '{"name":"테스트 방"}', '{}', 1);
                INSERT INTO chat_logs VALUES
                  (1, 1, 0, 900, 100,
                   '{"feedType":1,"members":[{"userId":200,"nickName":"길동"}]}',
                  '{}', '{"enc":0,"isMine":true}', 1000, NULL);
                INSERT INTO chat_logs VALUES
                  (2, 2, 52, 900, 200, '',
                   '{"type":"vr_invite","callId":"345","csIP":"203.0.113.5","csIP6":"2001:db8::5","csPort":17000}',
                   '{}', 1001, NULL);
                "#,
                )
                .unwrap();
        }
        {
            let secondary = Connection::open(databases.join("KakaoTalk2.db")).unwrap();
            secondary
                .execute_batch(
                    "CREATE TABLE friends (
                   id INTEGER PRIMARY KEY, name TEXT,
                   original_profile_image_url TEXT, enc INTEGER
                 );
                 CREATE TABLE open_link (
                   id INTEGER PRIMARY KEY, name TEXT, url TEXT, image_url TEXT, type INTEGER,
                   active INTEGER, expired INTEGER
                 );
                 CREATE TABLE open_chat_member (
                   _id INTEGER PRIMARY KEY, user_id INTEGER, involved_chat_id INTEGER,
                   profile_link_id INTEGER
                 );
                 INSERT INTO friends VALUES (200, '길동', NULL, 0);
                 INSERT INTO open_link VALUES
                   (700, '소유 오픈프로필', 'https://open.kakao.com/o/Profile700', NULL, 1, 1, 0);
                 INSERT INTO open_link VALUES
                   (701, '참여 중인 방', 'https://open.kakao.com/o/Room701', NULL, 2, 1, 0);
                 INSERT INTO open_link VALUES
                   (702, '만료 프로필', 'https://open.kakao.com/o/Expired702', NULL, 1, 1, 1);
                 INSERT INTO open_link VALUES
                   (703, '잘못된 링크', 'https://example.com/o/Invalid703', NULL, 2, 1, 0);
                 INSERT INTO open_chat_member VALUES (1, 300, 900, 8382);",
                )
                .unwrap();
        }
        {
            let profiles = Connection::open(databases.join("multi_profile_database.db")).unwrap();
            profiles
                .execute_batch(
                    "CREATE TABLE multi_profiles (
                       profileId TEXT PRIMARY KEY, nickName TEXT, isMain INTEGER,
                       \"order\" INTEGER, encryptType INTEGER
                     );
                     INSERT INTO multi_profiles VALUES ('main-profile', '나', 1, 0, 0);",
                )
                .unwrap();
        }
        let config = Settings {
            bind: "127.0.0.1:0".to_string(),
            kakao_path: Some(app_path),
            data_dir: directory.path().join("noa"),
            upload_dir: directory.path().join("uploads"),
            api_token: None,
            max_upload_bytes: 1024,
            poll_interval_ms: 100,
            snapshot_interval_ms: 500,
            send_interval_ms: 100,
            android_user_id: 0,
            calling_package: "com.android.shell".to_string(),
            file_provider_authority: None,
            image_max_dimension: 4096,
            jpeg_quality: 85,
            kakao_hook_enabled: true,
            chatonroom_interval_ms: 10_000,
            loco_history_limit: 1_000,
            iris_hook: crate::settings::IrisHookConfig {
                enabled: false,
                bridge_url: "http://127.0.0.1:4000/internal/iris/reply".to_string(),
                endpoint_bridge_url: "http://127.0.0.1:4000/internal/iris/endpoint".to_string(),
                endpoint_prefix: "/noa".to_string(),
                config_path: directory.path().join("iris-hook.json"),
                token: "test".to_string(),
                types: vec!["image".to_string()],
            },
        };
        let catalog = RoomCatalog::mount(&config).unwrap();
        assert_eq!(catalog.current_user_id(), 100);
        let rooms = catalog.snapshot().unwrap();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].name, "테스트 방");
        assert_eq!(rooms[0].member_count, 2);
        assert!(
            rooms[0]
                .members
                .iter()
                .any(|member| member.nickname == "길동")
        );
        assert_eq!(catalog.room_snapshot(900).unwrap().member_count, 2);
        assert!(catalog.room_has_member(900, 200).unwrap());
        assert_eq!(
            catalog.hide_message_target(900, 1).unwrap(),
            HideMessageTarget {
                message_type: 0,
                message: r#"{"feedType":1,"members":[{"userId":200,"nickName":"길동"}]}"#
                    .to_string(),
            }
        );
        assert!(matches!(
            catalog.hide_message_target(900, 999),
            Err(NoaError::NotFound(_))
        ));
        assert_eq!(
            catalog.voiceroom_join_info(900).unwrap(),
            VoiceroomJoinInfo {
                chat_id: 900,
                call_id: 345,
                host_v4: "203.0.113.5".to_string(),
                host_v6: "2001:db8::5".to_string(),
                port: 17_000,
            }
        );
        {
            let primary = Connection::open(databases.join("KakaoTalk.db")).unwrap();
            primary
                .execute(
                    "UPDATE chat_rooms SET active_member_ids = '[100]' WHERE id = 900",
                    [],
                )
                .unwrap();
            primary
                .execute(
                    "INSERT INTO chat_logs
                     (_id, id, type, chat_id, user_id, message, attachment, v, created_at)
                     VALUES (3, 3, 268435508, 900, 200, '', '{\"type\":\"vr_bye\"}', '{}', 1002)",
                    [],
                )
                .unwrap();
        }
        assert!(!catalog.room_has_member(900, 200).unwrap());
        assert_eq!(catalog.room_snapshot(900).unwrap().member_count, 1);
        assert!(matches!(
            catalog.voiceroom_join_info(900),
            Err(NoaError::NotFound(_))
        ));
        let profiles = catalog.owned_profiles().unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].profile_id, "main-profile");
        assert_eq!(profiles[0].nickname, "나");
        assert!(profiles[0].is_main);
        assert_eq!(profiles[0].kind, OwnedProfileKind::Kakao);
        assert_eq!(profiles[1].profile_id, "700");
        assert_eq!(profiles[1].kind, OwnedProfileKind::OpenProfile);
        assert_eq!(
            catalog.open_profile_share_target(700).unwrap(),
            Some("https://open.kakao.com/o/Profile700".to_string())
        );
        assert!(catalog.open_profile_share_target(0).is_err());
        assert!(catalog.open_profile_share_target(999).is_err());
        assert!(catalog.open_profile_share_target(702).is_err());
        assert!(catalog.open_profile_share_target(703).is_err());
        assert!(catalog.open_profile_share_target(701).is_err());
        assert_eq!(catalog.open_profile_share_target(8382).unwrap(), None);
        assert_eq!(
            catalog.member_open_profile_link_id(900, 300).unwrap(),
            Some(8382)
        );
        assert_eq!(catalog.member_open_profile_link_id(900, 200).unwrap(), None);
        assert!(catalog.member_open_profile_link_id(0, 300).is_err());
        let (last, feeds) = catalog.changes_since(0, 10).unwrap();
        assert_eq!(last, 1);
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].kind, EventKind::Joined);
        let queued = catalog
            .enqueue_custom(CustomMessageDraft {
                message_type: 1,
                chat_id: 900,
                thread_id: None,
                scope: 1,
                message: "custom".to_string(),
                attachment: "{}".to_string(),
                created_at: None,
                client_message_id: Some(12345),
                supplement: None,
                metadata: None,
                is_silence: 0,
            })
            .unwrap();
        assert!(queued.row_id > 0);
        assert_eq!(queued.client_message_id, 12345);
        assert_eq!(
            catalog.delivery_state(12345).unwrap(),
            DeliveryState::Waiting
        );
    }
}
