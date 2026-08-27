# 2. 데이터 관찰과 이벤트 처리

이전: [전체 구조와 실행 흐름](1-architecture.md) · [README로 돌아가기](../README.md) · 다음: [메시지·파일 전송 원리](3-delivery.md)

## 데이터 소스

Noa는 KakaoTalk 앱 데이터 디렉터리 아래의 DB를 직접 읽습니다. 주요 Room과 메시지 정보는 `KakaoTalk.db`, 오픈채팅 멤버·링크·프로필 정보는 `KakaoTalk2.db`, 일반·멀티 프로필은 `multi_profile_database.db`에서 가져옵니다. 스키마 차이를 감안해 필요한 테이블과 열을 런타임에 확인하며, 필수 구조가 없으면 추측해서 진행하지 않고 명시적 DB 오류를 반환합니다.

Android 사용자별 기본 경로를 순서대로 검사하고 `KakaoTalk.db`가 실제로 있는 첫 경로를 선택합니다. 자동 탐색이 맞지 않는 환경에서는 `NOA_KAKAO_PATH`로 앱 데이터 루트를 지정합니다.

## 암호화 DB 열기

KakaoTalk DB는 SQLCipher 설정이 필요할 수 있습니다. catalog는 앱 설정에서 키 재료를 읽고 호환되는 cipher 설정으로 읽기 연결을 구성합니다. 주 DB와 보조 DB는 alias로 attach하여 하나의 조회에서 Room, 멤버, 오픈링크 정보를 결합합니다.

조회 연결은 기본적으로 읽기 전용입니다. 예외는 custom 메시지를 발신하기 위해 `chat_sending_logs`에 행을 넣고 전송 결과를 확인하는 경로이며, 이 작업은 제한된 쓰기 연결에서 수행됩니다.

## Room 스냅샷

Room 스냅샷은 다음 공개 모델로 정규화됩니다.

```json
{
  "chatId":"123456789",
  "name":"방 이름",
  "roomType":"OM",
  "memberCount":2,
  "members":[
    {
      "userId":"111",
      "nickname":"사용자",
      "profileImageUrl":null,
      "isMine":false
    }
  ]
}
```

식별자는 DB에서는 정수이지만 HTTP 경계에서는 문자열로 변환합니다. JavaScript 클라이언트가 64비트 ID를 부정확하게 반올림하는 문제를 피하기 위해서입니다. Room 캐시는 API 응답과 대상 검증에 사용되지만, 강퇴처럼 정확성이 중요한 작업은 직전에 DB에서 해당 방을 다시 조회합니다.

## 변경 감지

Kakao 프로세스 안의 Room watcher가 Room DB 무효화 콜백을 감지해 Noa로 보냅니다. 데이터베이스와 테이블에 따라 최소 갱신 범위를 선택합니다.

| 변경 | 수행 작업 |
|---|---|
| `master.chat_logs` | feed 증분 조회 |
| `master.chat_rooms` | Room 스냅샷 갱신 |
| `secondary.open_chat_member` | Room 스냅샷 갱신 |
| `secondary.open_link` | Room 스냅샷 갱신 |
| `secondary.open_profile` | Room 스냅샷 갱신 |

연속 무효화는 50ms 동안 모아 같은 DB를 반복 조회하지 않도록 합칩니다. 콜백이 누락되거나 에이전트가 비활성인 상황을 위해 feed와 전체 스냅샷의 주기적 안전 폴링도 유지합니다.

## 이벤트 생성

변경 이벤트는 두 경로에서 생성됩니다.

1. Feed 증분 조회는 KakaoTalk이 기록한 입장·퇴장 계열 메시지를 cursor 이후부터 읽습니다.
2. 스냅샷 비교는 이전/현재 멤버 집합을 비교해 입장, 퇴장, 닉네임 변경을 계산합니다.

두 경로가 같은 사건을 보고할 수 있으므로 감사 DB는 source key와 짧은 시간 범위 비교로 중복을 제거합니다. 저장된 이벤트의 `source`는 보통 `feed` 또는 `snapshot`이며 `kind`는 `joined`, `left`, `kicked`, `nickname_changed` 중 하나입니다.

## 감사 저장소와 실시간 전파

Noa 자체 감사 기록은 `$NOA_DATA_DIR/noa.db`의 `room_events`에 저장합니다. WAL 모드와 조회 패턴별 인덱스를 사용하고, `/api/events`에서 최신순으로 최대 1000개까지 읽습니다.

새 이벤트가 저장되면 Tokio broadcast 채널에도 게시됩니다. `/api/events/stream`은 이 채널을 SSE로 전달합니다. 느린 구독자가 broadcast 용량을 초과하면 지나간 실시간 항목은 건너뛸 수 있으므로 완전한 이력이 필요하면 `/api/events`를 다시 조회해야 합니다.

## 사후 조건 검증

관리 작업은 가능한 경우 DB를 최종 진실로 사용합니다.

- 강퇴: 최대 8초 동안 대상 `userId`가 `active_member_ids`에서 제거됐는지 확인합니다.
- custom 전송: `client_message_id`가 대기 테이블에서 실제 채팅 로그로 이동했는지 확인합니다.
- 프로필 공유: 요청한 `linkId`, DB URL, 내부 조회 또는 UI 복사 결과의 형식을 대조합니다.
- 오픈채팅 입장: 반환된 Room의 실제 `linkId`가 요청 링크와 일치하는지 확인합니다.

검증 시간 내 결론을 내릴 수 없으면 중복 실행 위험이 있으므로 성공으로 가장하지 않고 오류를 반환합니다.
