# 6. API 레퍼런스

[README로 돌아가기](../README.md)

현재 소스 기준의 Noa HTTP API와 Iris 연동 명세입니다. 배포 버전별 지원 범위와 카카오톡 호환성은 [지원 환경](../README.md#지원-환경)을 참고합니다.

- [공통 규칙](#api-공통-규칙)
- [상태 및 조회](#상태-및-조회-api)
- [LOCO](#loco-api)
- [메시지·파일 전송](#전송-api)
- [오픈채팅 및 방 관리](#오픈채팅-및-방-관리-api)
- [VOX·PCM 송출](#vox-보이스톡-및-pcm-송출-api)
- [Iris 연동](#iris-api)

## API 공통 규칙

Noa의 기본 URL은 `http://127.0.0.1:4000`입니다. `NOA_API_TOKEN`을 설정하면 `/health`, `/`, `/dashboard`, `/loco`를 제외한 공개 API에 인증이 적용됩니다. 다음 헤더 중 하나를 사용합니다.

```http
Authorization: Bearer API_TOKEN
```

`X-Noa-Token: API_TOKEN` 헤더도 지원합니다. 설치 스크립트는 토큰을 생성해 `.noa-device.env`에 저장합니다. 서버를 직접 실행하면서 토큰을 설정하지 않으면 공개 API 인증이 비활성화됩니다.

JSON 요청에는 `Content-Type: application/json`을 지정합니다. 애플리케이션 오류는 다음 형식으로 반환합니다.

```json
{"error":"오류 설명"}
```

| 상태 | 의미 |
|---:|---|
| `400` | 요청 형식 또는 값 오류 |
| `401` | 인증 실패 |
| `403` | 작업 권한 없음 또는 카카오톡 서버의 작업 거부 |
| `404` | 대상 또는 기능을 찾을 수 없음 |
| `409` | VOX 세션 종료 또는 대상 세션 변경 |
| `503` | Android/에이전트 기능을 사용할 수 없음 |
| `500` | 데이터베이스 또는 내부 처리 오류 |

`chatId`, `userId`, `logId` 등 카카오톡 식별자는 응답에서 문자열로 반환합니다. 이벤트 `id`, LOCO `packetId`, custom 전송의 `rowId`·`clientMessageId`는 숫자입니다.

응답 예시는 구조를 설명하기 위한 값입니다. 기능별 성공 응답은 Intent 실행, 앱 내부 요청 수락, DB 확인, 소켓 쓰기 등 서로 다른 완료 시점을 나타내므로 각 엔드포인트의 설명을 확인합니다.

## 상태 및 조회 API

### `GET /health`

인증 없이 서버의 실행 상태와 버전을 조회합니다.

```json
{"ok":true,"service":"noa","version":"1.4.3"}
```

### `GET /api/status`

서버와 카카오톡의 연동 상태를 조회합니다.

```json
{
  "version":"1.4.3",
  "revision":"<git-commit-sha>",
  "databaseAvailable":true,
  "androidAvailable":true,
  "authenticationEnabled":true,
  "roomCount":3,
  "currentUserId":"123456789",
  "maxUploadBytes":67108864,
  "irisHookEnabled":true,
  "irisHookActive":true,
  "irisEndpointPrefix":"/noa",
  "kakaoHookEnabled":true,
  "kakaoHookActive":true,
  "voxControlActive":true,
  "voxAudioActive":true
}
```

`Enabled`는 설정, `Active`는 에이전트 연결 상태입니다.
`androidAvailable`은 별도 프로세스의 ART 사전 검사를 통과한 뒤 JNI 전송 계층까지
준비됐는지를 나타냅니다. ART 사전 검사 프로세스가 충돌하거나 15초 안에 끝나지
않으면 Noa 서버와 후킹 기능은 제한 모드로 계속 실행되고 이 값만 `false`가 됩니다.
초기화 단계는 `/data/local/tmp/noa.log`의 `stage` 필드에서 확인할 수 있습니다.

### `GET /api/rooms`

참여 중인 채팅방 목록을 조회합니다.

```json
[
  {
    "chatId":"123456789",
    "name":"테스트 방",
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
]
```

### `GET /api/rooms/{chatId}`

지정한 채팅방을 조회합니다. 채팅방을 찾을 수 없으면 `404`를 반환합니다.

### `GET /api/events`

참여자 이벤트를 최신순으로 반환합니다.

| 쿼리 매개변수 | 타입 | 필수 | 설명 |
|---|---|---:|---|
| `chatId` | 정수 문자열 | 아니요 | 특정 방만 조회 |
| `userId` | 양의 정수 문자열 | 아니요 | 특정 사용자 조회. `chatId` 필요 |
| `limit` | 정수 | 아니요 | 기본 `200`, 실제 범위 `1..1000` |

```json
[
  {
    "id":1,
    "chatId":"123456789",
    "roomName":"테스트 방",
    "kind":"joined",
    "userId":"111",
    "nickname":"사용자",
    "previousNickname":null,
    "occurredAt":1720000000,
    "source":"feed"
  }
]
```

`kind`는 `joined`, `left`, `kicked`, `nickname_changed` 중 하나입니다.

### `GET /api/events/stream`

새 참여자 이벤트를 Server-Sent Events(SSE)로 전달합니다.

```text
: connected

data: {"id":2,"chatId":"123456789","roomName":"테스트 방","kind":"left","userId":"111","nickname":"사용자","previousNickname":null,"occurredAt":1720000010,"source":"snapshot"}

```

## LOCO API

### `GET /api/loco?limit=500`

최근 LOCO 패킷을 반환합니다. `limit` 기본값은 `500`, 범위는 `1..10000`입니다.

```json
[
  {
    "id":1,
    "direction":"receive",
    "method":"MSG",
    "packetId":10,
    "status":0,
    "bodyLength":128,
    "body":"{...}",
    "capturedAt":1720000000000
  }
]
```

### `POST /api/loco/send`

로그인된 카카오톡의 기존 LOCO 연결로 패킷을 직접 발신합니다. `KAKAO_HOOK_ENABLED=true`와 활성 후킹 연결이 필요하며, 후킹 이후 앱의 정상 송신이 한 번 관찰되어야 합니다.

```bash
curl -X POST http://127.0.0.1:4000/api/loco/send \
  -H 'Authorization: Bearer API_TOKEN' \
  -H 'Content-Type: application/json' \
  -d '{"method":"PING","body":{}}'
```

`method`는 앱의 LOCO 메서드 목록에 포함된 대문자 메서드명(1~11자), `body`는 JSON 객체입니다. 본문의 직렬화된 JSON은 64KiB, 앱이 인코딩한 BSON은 128KiB, 중첩은 32단계까지 지원합니다. 메서드별 필수 필드와 서버 권한 검사는 카카오톡 서버가 결정합니다.

| 본문 값 | BSON 타입 |
|---|---|
| 일반 정수 | 범위에 따라 int32 또는 int64 |
| 소수 | double |
| `{"$numberLong":"1234567890123456789"}` | 정확한 int64; ID 값에 권장 |
| `{"$numberInt":"1"}` | int32 |
| `{"$numberDouble":"1.5"}` | double |
| `{"$binary":"AAH/"}` | Base64를 디코딩한 바이너리 |
| 객체·배열·문자열·bool·null | 대응 BSON 값 |

타입 지정 객체는 값 위치에서 단독 필드로 사용합니다. 이 표는 지원하는 형식 전체이며 MongoDB Extended JSON 전체를 지원하는 것은 아닙니다. BSON 문서 키에 NUL은 허용하지 않습니다.

응답 예시:

```json
{"ok":true,"method":"PING","packetId":1000,"bodyLength":5,"verification":"socket-write","serverConfirmed":false}
```

패킷 ID와 BSON 길이는 앱이 계산하며, 요청의 `status`는 `0`으로 설정됩니다. `ok`는 소켓 쓰기·flush 호출 완료이며 서버 처리 성공을 의미하지 않습니다. 이 API는 응답 대기 객체를 등록하지 않으므로 대응 응답이 `/api/loco` 수신 기록에 나타난다고 보장하지 않습니다. 송신 시도는 기존 LOCO 캡처 경로를 사용합니다. 쓰기 오류·타임아웃에는 전송 여부가 불확실할 수 있으며 자동 재전송하지 않습니다.

실제 기기에서의 서버 송수신은 아직 검증하지 않았습니다.

## 전송 API

### `POST /send`

이미지와 파일을 전송합니다. 업로드 크기는 `NOA_MAX_UPLOAD_BYTES` 설정으로 제한합니다.

Multipart 요청:

| 필드 | 타입 | 필수 | 설명 |
|---|---|---:|---|
| `chatId` 또는 `chat_id` | 정수 문자열 | 예 | 대상 방 |
| `file` 또는 `data` | binary | 예 | 파일 하나 |

```bash
curl -X POST http://127.0.0.1:4000/send \
  -H 'Authorization: Bearer API_TOKEN' \
  -F 'chatId=123456789' \
  -F 'file=@photo.png'
```

원시 바이너리는 `POST /send?chatId=123456789&filename=photo.png`로 전송합니다. 파일명은 `filename` 쿼리 매개변수 또는 `X-Filename` 헤더로 지정합니다.

```json
{
  "ok":true,
  "chatId":"123456789",
  "file":{
    "fileName":"photo.png",
    "mimeType":"image/png",
    "originalBytes":1024,
    "storedBytes":1024,
    "optimized":false
  },
  "message":"KakaoTalk 공유 Intent를 실행했습니다"
}
```

### `POST /send/text`

`threadId`는 생략하거나 `null`로 지정할 수 있습니다.

```json
{"chatId":"123456789","text":"안녕하세요","threadId":null}
```

```json
{"ok":true,"chatId":"123456789","message":"KakaoTalk 답장 Intent를 실행했습니다"}
```

## 오픈채팅 및 방 관리 API

### `GET /api/open-chat/profiles`

현재 계정이 사용할 수 있는 프로필 목록을 조회합니다.

```json
{
  "ok":true,
  "profiles":[
    {
      "profileId":"700",
      "nickname":"선택 프로필",
      "profileImageUrl":null,
      "kind":"openProfile",
      "isMain":false
    }
  ]
}
```

`kind`는 `kakao` 또는 `openProfile`입니다.

### `POST /api/open-chat/profiles/share`

현재 계정의 오픈프로필 공유 URL을 조회합니다. `mode`는 `auto`(기본값), `hook`, `accessibility` 중 하나입니다.

```json
{"linkId":"700","mode":"auto"}
```

```json
{
  "ok":true,
  "linkId":"700",
  "url":"https://open.kakao.com/o/PROFILE_TOKEN",
  "mode":"hook",
  "verified":true,
  "verification":"database+hook"
}
```

### `POST /api/open-chat/profiles/share-member`

채팅방 참여자의 오픈프로필 공유 URL을 조회합니다.

```json
{"chatId":"123456789","userId":"111","mode":"accessibility"}
```

```json
{
  "ok":true,
  "chatId":"123456789",
  "userId":"111",
  "linkId":null,
  "url":"https://open.kakao.com/me/PROFILE_TOKEN",
  "mode":"accessibility",
  "verified":true,
  "verification":"database-member+ui+clipboard"
}
```

### `POST /api/open-chat/join`

오픈채팅에 입장합니다. `profileId`를 생략하면 첫 소유 프로필을 사용합니다.

```json
{"url":"https://open.kakao.com/o/OPEN_CHAT_ID","profileId":"700"}
```

```json
{
  "ok":true,
  "roomName":"오픈채팅방",
  "profileId":"700",
  "profile":"선택 프로필",
  "profileApplied":true,
  "mode":"hook",
  "message":"오픈채팅 입장을 완료했습니다"
}
```

### `POST /api/rooms/{chatId}/kick`

`nickname` 또는 `userId`로 참여자를 강퇴합니다. 자기 자신은 대상이 될 수 없습니다.
카카오톡 서버가 권한이나 대상 역할 때문에 요청을 거부하면 `403`과 KICKMEM 응답의 `status`, `errMsg`를 반환합니다.

```json
{"userId":"111"}
```

```json
{
  "ok":true,
  "verified":true,
  "verification":"database",
  "chatId":"123456789",
  "roomName":"오픈채팅방",
  "userId":"111",
  "nickname":"사용자",
  "message":"참여자 강퇴를 완료했습니다"
}
```

### `POST /api/rooms/{chatId}/leave`

지정한 채팅방에서 나갑니다. 요청 본문은 필요하지 않습니다.

```json
{
  "ok":true,
  "chatId":"123456789",
  "roomName":"오픈채팅방",
  "message":"채팅방 나가기를 완료했습니다"
}
```

### `POST /api/rooms/{chatId}/messages/{logId}/hide`

오픈채팅 방장이 지정한 메시지를 모든 참여자에게서 가립니다. 요청 본문은 없으며
KakaoTalk 후킹 모드가 필요합니다. 응답의 `accepted`는 카카오톡이 내부 요청을
수락했다는 뜻이며, 서버 반영 완료를 보장하는 값은 아닙니다.

```json
{
  "ok":true,
  "accepted":true,
  "verified":false,
  "verification":"kakao-dispatch",
  "chatId":"123456789",
  "logId":"987654321",
  "message":"메시지 가리기 요청을 KakaoTalk에 전달했습니다"
}
```

## VOX 보이스톡 및 PCM 송출 API

VOX API는 `KAKAO_HOOK_ENABLED=true`와 `RECORD_AUDIO` 권한이 필요합니다.

### 일반 보이스톡

`peerIds`를 생략하면 현재 사용자를 제외한 활성 참여자 전체에 보이스톡을 겁니다.

```bash
curl -X POST http://127.0.0.1:4000/api/vox/voice-talk \
  -H 'Authorization: Bearer API_TOKEN' \
  -H 'Content-Type: application/json' \
  -d '{"chatId":"123456789","peerIds":["111"]}'
```

### 오픈채팅 보이스룸

`POST /api/vox/voice-rooms`는 OpenMulti(`OM`) 방에 보이스룸을 만듭니다.

```json
{"chatId":"123456789","title":"방송"}
```

참여는 `POST /api/vox/voice-rooms/join`에 `chatId`를 보냅니다.

```json
{"chatId":"123456789"}
```

세션을 종료하려면 `POST /api/vox/leave`를 호출합니다.

```json
{"chatId":"123456789","kind":"cecall"}
```

`kind`는 보이스톡의 경우 `cecall`, 보이스룸의 경우 `voiceroom`입니다. 현재 상태는 `GET /api/vox/status`로 조회합니다.

### PCM 음원 송출

입력 형식은 헤더 없는 `s16le` PCM(48 kHz, 모노)입니다. `replace`는 마이크 입력을 대체하고, `mix`는 마이크 입력에 PCM을 혼합합니다.

```bash
ffmpeg -i music.mp3 -f s16le -ar 48000 -ac 1 - \
  | curl --http1.1 -X POST --upload-file - \
      'http://127.0.0.1:4000/api/vox/audio/stream?mode=replace&kind=voiceroom&chatId=123456789' \
      -H 'Authorization: Bearer API_TOKEN' \
      -H 'Content-Type: application/octet-stream'
```

Noa는 입력을 초당 96,000바이트 속도에 맞춰 전송하고 VOX 세션을 250ms 간격으로 확인합니다. 대상 세션이 종료되거나 다른 방으로 바뀌면 PCM 주입을 중지하고 HTTP `409`를 반환합니다. `kind`는 `cecall` 또는 `voiceroom`이며 `chatId`와 함께 지정할 수 있습니다. 단일 `/api/vox/audio` 요청은 최대 96,000바이트의 완전한 16-bit sample이어야 합니다.

## Iris API

`file`, `markdown`, `custom`은 Iris `POST /reply`로 전송합니다.

### Iris `POST /reply`

Noa가 처리할 확장 타입은 `NOA_IRIS_HOOK_TYPES`로 지정합니다. Iris의 기본 전송 타입은 Iris가 처리합니다. Iris `v0.33`의 `media`, `video`, `audio`도 그대로 전달됩니다.

Iris `v0.33`의 `/reply` 성공 응답은 `{"success":true,"message":"Enqueued"}`입니다. 이는 비동기 처리 접수를 뜻하며, Noa 전송 완료나 DB 확인 결과를 보장하지 않습니다.

#### Markdown

카카오톡 26.9 이상에서는 Markdown을 RichText로 변환해 내부 재전송 경로로 전송하며, `KAKAO_HOOK_ENABLED=true`가 필요합니다. 이전 버전에서는 Markdown 원문과 `{"markdown":true}` attachment를 기존 공유 Intent로 전송합니다. 구버전 경로는 KakaoTalk 후킹 없이도 사용할 수 있습니다.

```json
{"type":"markdown","room":"123456789","data":"**굵은 메시지**"}
```

RichText 경로에서는 `**굵게**`/`__굵게__`, `*기울임*`/`_기울임_`, `~~취소선~~`을 본문과 서식 범위로 변환합니다. 중첩 서식과 한글·이모지를 지원하며, 제목·목록·링크·코드 등 그 밖의 문법은 원문으로 남깁니다. 이 경로의 Noa 내부 처리 성공은 같은 `client_message_id`의 `chat_logs` 기록 확인을 뜻합니다. 실제 수신·표시는 기기 확인이 필요합니다.

구버전 경로의 처리 성공은 Intent 실행 완료를 뜻합니다. 버전 조회 실패나 RichText 지원 버전의 후킹 오류는 오류로 반환하며, Intent로 재시도하지 않습니다. Iris의 비동기 `Enqueued` 응답에는 이후 Noa의 지원 여부 검사나 전송 결과가 포함되지 않습니다.

#### 파일

Android 파일 경로 또는 Base64/Data URI로 파일을 전송합니다.

```json
{"type":"file","room":"123456789","path":"/sdcard/Download/report.pdf"}
```

```json
{"type":"file","room":"123456789","data":"data:application/pdf;name=report.pdf;base64,..."}
```

`file`의 `data`가 객체나 배열이고 `path`가 없으면 Iris의 미디어 처리기로 전달합니다. Iris `v0.33`에서 추가된 파일명·MIME 지정 형식을 사용할 수 있습니다. Base64 문자열과 명시적인 `path`는 `file` 후킹이 활성화된 경우 Noa가 처리합니다.

#### Custom

메시지 타입과 attachment를 지정해 전송합니다.

```json
{
  "type":"custom",
  "room":"123456789",
  "data":{
    "type":1,
    "message":"메시지",
    "attachment":{},
    "chat_id":"123456789",
    "thread_id":null,
    "scope":1,
    "v":null,
    "is_silence":0
  }
}
```

`data.type`의 범위는 `1..65535`입니다. `data.chat_id`를 지정하면 `room`과 같아야 합니다.

Noa 내부 브리지의 `file` 및 구버전 `markdown` 처리 완료 결과:

```json
{"success":true,"message":"success"}
```

Noa 내부 브리지의 RichText `markdown`과 `custom` 처리 완료 결과:

```json
{
  "success":true,
  "message":"success",
  "verified":true,
  "verification":"database",
  "chatId":"123456789",
  "rowId":10,
  "clientMessageId":20
}
```

이 내부 결과는 Iris `/reply`의 HTTP 응답으로 전달되지 않습니다.

### Iris 확장 엔드포인트

기본 URL은 `http://127.0.0.1:3000/noa`입니다. 경로 접두사는 `NOA_IRIS_ENDPOINT_PREFIX`로 변경합니다.

| 메서드 | 경로 | Noa API 대응 |
|---|---|---|
| `GET` | `/noa/health` | 확장 게이트웨이 상태 |
| `GET` | `/noa/open-chat/profiles` | `GET /api/open-chat/profiles` |
| `POST` | `/noa/open-chat/profiles/share` | `POST /api/open-chat/profiles/share` |
| `POST` | `/noa/open-chat/profiles/share-member` | `POST /api/open-chat/profiles/share-member` |
| `POST` | `/noa/open-chat/join` | `POST /api/open-chat/join` |
| `POST` | `/noa/rooms/{chatId}/kick` | `POST /api/rooms/{chatId}/kick` |
| `POST` | `/noa/rooms/{chatId}/messages/{logId}/hide` | `POST /api/rooms/{chatId}/messages/{logId}/hide` |
| `POST` | `/noa/rooms/{chatId}/leave` | `POST /api/rooms/{chatId}/leave` |
| `GET` | `/noa/vox/status` | `GET /api/vox/status` |
| `POST` | `/noa/vox/voice-talk` | `POST /api/vox/voice-talk` |
| `POST` | `/noa/vox/voice-rooms` | `POST /api/vox/voice-rooms` |
| `POST` | `/noa/vox/voice-rooms/join` | `POST /api/vox/voice-rooms/join` |
| `POST` | `/noa/vox/leave` | `POST /api/vox/leave` |
| `POST` | `/noa/vox/audio/start` | `POST /api/vox/audio/start` |
| `POST` | `/noa/vox/audio` | `POST /api/vox/audio` |
| `POST` | `/noa/vox/audio/stream` | `POST /api/vox/audio/stream` |
| `POST` | `/noa/vox/audio/stop` | `POST /api/vox/audio/stop` |

본문은 대응하는 Noa API와 동일하며 Iris가 내부 인증을 추가합니다.
PCM 본문도 `application/octet-stream` 그대로 전송하며 Base64 변환은 내부 브리지에서만 수행합니다. Iris의 Ktor gateway는 요청 본문을 전달 전에 메모리에 모으므로 장시간 실시간 입력은 `/noa/vox/audio`에 작은 PCM 청크를 순차 전송하거나 Noa의 `/api/vox/audio/stream`을 직접 사용합니다.
