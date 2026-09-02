# noa

<p align="center">
  <img src="assets/noa-cover.jpg" alt="Noa KakaoTalk illustration" width="480">
</p>

Noa는 루팅된 Android 또는 Redroid에서 KakaoTalk 조회·전송·오픈채팅 관리·이벤트 감사를 제공하는 네이티브 서비스입니다. ARM64, ARMv7, x86, x86_64를 지원합니다.

> **주의:** 내부 데이터베이스, 비공개 API와 프로세스 후킹을 사용합니다. 소유·관리 권한이 있는 테스트 환경에서만 사용하고 관련 법령과 KakaoTalk 정책을 준수하십시오. Noa는 Kakao의 공식 제품이 아니며 어떠한 보증도 제공하지 않습니다. 소스·바이너리 재배포는 금지되며 자세한 조건은 [Noa License 1.0](LICENSE)을 따릅니다.

## 라이선스 및 문의

- 제작자·저작권자: Entworacy
- 이메일: [entworacy@gmail.com](mailto:entworacy@gmail.com)
- 라이선스: [Noa License 1.0](LICENSE)

개인·조직 내부 사용과 사적 수정은 허용되지만, 저작권자의 사전 서면 허가 없이 다음 행위를 할 수 없습니다.

- 소스·바이너리·수정본 재배포
- Noa 실행 바이너리 자체를 다른 프로젝트·제품·설치 파일·패키지에 포함하여 제3자에게 배포
- 소프트웨어 또는 그 사본의 유상 판매·재판매·대여·유상 설치

제3자 구성 요소와 상호운용 참고 사항은 [NOTICE](NOTICE)와 [licenses](licenses)를 확인하십시오.

## 지원 환경

| 항목 | 내용 |
|---|---|
| 최적화·검증 기준 | KakaoTalk Android `26.6.3` |
| 실행 환경 | Android API 26 이상, root 권한 필요 |
| 기본 동작 | `KAKAO_HOOK_ENABLED=true` |
| 후킹 방식 | Frida Core + LSPlant |
| 비후킹 모드 | Android Intent + UiAutomator |

다른 KakaoTalk 버전은 호환성을 보장하지 않습니다. 세부 사항은 [후킹 및 접근성 에이전트](docs/4-agents.md)를 참고하십시오.

## 구조

Noa는 KakaoTalk DB, Android Intent, 네이티브 후킹과 UiAutomator를 기능별로 사용합니다.

1. [전체 구조와 실행 흐름](docs/1-architecture.md)
2. [데이터 관찰과 이벤트 처리](docs/2-data-and-events.md)
3. [메시지·파일 전송 원리](docs/3-delivery.md)
4. [후킹 및 접근성 에이전트](docs/4-agents.md)

## 설치

사전 요구 사항:

- Android API 26 이상
- ADB로 연결할 수 있는 Linux 환경
- `bash`, `curl`, `python3`, `sha256sum`

```bash
git clone --depth 1 https://github.com/entworacy/noa.git
cd noa
adb connect 127.0.0.1:5555
./scripts/iris-noa install --serial 127.0.0.1:5555
./scripts/iris-noa up
```

ADB 주소는 환경에 맞게 변경합니다. `--iris-control PATH`로 기존 `iris_control`을 지정할 수 있습니다. 설치 설정과 API 토큰은 `.noa-device.env`에 저장됩니다.

```bash
./scripts/iris-noa status
./scripts/iris-noa stop
```

기본 주소는 Noa `http://127.0.0.1:4000`, Iris `http://127.0.0.1:3000`입니다.

## Python 클라이언트

Python 3.10 이상에서 [`irispy-noa-client`](https://github.com/entworacy/irispy-noa-client)를 사용합니다.

```bash
pip uninstall -y irispy-client
pip install git+https://github.com/entworacy/irispy-noa-client.git
```

```python
from iris import Bot

bot = Bot(
    "127.0.0.1:3000",
    noa_prefix="/noa",
    timeout=130.0,
)
```

Iris `/noa/...` 요청에는 에이전트가 내부 인증을 추가합니다.

## API 공통 규칙

기본 URL은 `http://127.0.0.1:4000`입니다. `/health`, `/`, `/dashboard`, `/loco` 외 공개 API는 다음 헤더로 인증합니다.

```http
Authorization: Bearer API_TOKEN
X-Noa-Token: API_TOKEN
```

JSON 요청은 `Content-Type: application/json`, 오류 응답은 다음 형식입니다.

```json
{"error":"오류 설명"}
```

| 상태 | 의미 |
|---:|---|
| `400` | 요청 형식 또는 값 오류 |
| `401` | 인증 실패 |
| `403` | 작업 권한 없음 또는 카카오 서버의 작업 거부 |
| `404` | 대상 또는 기능을 찾을 수 없음 |
| `409` | VOX 세션 종료 또는 대상 세션 변경 |
| `503` | Android/에이전트 기능을 사용할 수 없음 |
| `500` | 데이터베이스 또는 내부 처리 오류 |

ID 필드는 응답에서 문자열로 반환됩니다.

## 상태 및 조회 API

### `GET /health`

인증 없는 상태 확인입니다.

```json
{"ok":true,"service":"noa","version":"1.3.9"}
```

### `GET /api/status`

서버와 KakaoTalk 연동 상태입니다.

```json
{
  "version":"1.3.9",
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
  "kakaoHookActive":true
}
```

`Enabled`는 설정, `Active`는 에이전트 연결 상태입니다.
`androidAvailable`은 별도 프로세스의 ART 사전 검사를 통과한 뒤 JNI 전송 계층까지
준비됐는지를 나타냅니다. ART 사전 검사 프로세스가 충돌하거나 15초 안에 끝나지
않으면 Noa 서버와 후킹 기능은 제한 모드로 계속 실행되고 이 값만 `false`가 됩니다.
초기화 단계는 `/data/local/tmp/noa.log`의 `stage` 필드에서 확인할 수 있습니다.

### `GET /api/rooms`

참여 중인 채팅방 목록입니다.

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

Room 한 건을 반환하며, 없으면 `404`입니다.

### `GET /api/events`

참여자 이벤트를 최신순으로 반환합니다.

| Query | 타입 | 필수 | 설명 |
|---|---|---:|---|
| `chatId` | integer string | 아니요 | 특정 방만 조회 |
| `userId` | positive integer string | 아니요 | 특정 사용자 조회. `chatId` 필요 |
| `limit` | integer | 아니요 | 기본 `200`, 실제 범위 `1..1000` |

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

새 RoomEvent를 SSE로 전달합니다.

```text
: connected

data: {"id":2,"chatId":"123456789","roomName":"테스트 방","kind":"left","userId":"111","nickname":"사용자","previousNickname":null,"occurredAt":1720000010,"source":"snapshot"}

```

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

## 전송 API

### `POST /send`

파일 전송 API입니다. 최대 크기는 `NOA_MAX_UPLOAD_BYTES`입니다.

Multipart 요청:

| 필드 | 타입 | 필수 | 설명 |
|---|---|---:|---|
| `chatId` 또는 `chat_id` | integer string | 예 | 대상 방 |
| `file` 또는 `data` | binary | 예 | 파일 하나 |

```bash
curl -X POST http://127.0.0.1:4000/send \
  -H 'Authorization: Bearer API_TOKEN' \
  -F 'chatId=123456789' \
  -F 'file=@photo.png'
```

원시 바이너리는 `POST /send?chatId=123456789&filename=photo.png`로 보내며 파일명은 `filename` query 또는 `X-Filename` 헤더로 지정합니다.

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

사용 가능한 소유 프로필 목록입니다.

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

소유 오픈프로필의 공유 URL입니다. `mode`: `auto`(기본값), `hook`, `accessibility`.

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

방 참여자의 오픈프로필 공유 URL입니다.

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
카카오 서버가 권한이나 대상 역할 때문에 요청을 거부하면 `403`과 KICKMEM 응답의 `status`, `errMsg`를 반환합니다.

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

요청 본문 없이 방에서 나갑니다.

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
KakaoTalk 후킹 모드가 필요합니다. 응답의 `accepted`는 KakaoTalk가 내부 요청을
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

종료:

```json
{"chatId":"123456789","kind":"cecall"}
```

`kind`: `cecall` 또는 `voiceroom`. 상태 조회: `GET /api/vox/status`.

### PCM 음원 송출

입력: header 없는 `s16le`, 48 kHz, mono. 모드: `replace` 또는 `mix`.

```bash
ffmpeg -i music.mp3 -f s16le -ar 48000 -ac 1 - \
  | curl --http1.1 -X POST --upload-file - \
      'http://127.0.0.1:4000/api/vox/audio/stream?mode=replace&kind=voiceroom&chatId=123456789' \
      -H 'Authorization: Bearer API_TOKEN' \
      -H 'Content-Type: application/octet-stream'
```

Noa가 입력을 96,000 bytes/s로 pacing하며 VOX 세션을 250ms 간격으로 확인합니다. 대상 세션이 종료되거나 다른 방으로 바뀌면 PCM 주입을 중지하고 HTTP `409`를 반환합니다. `kind`는 `cecall` 또는 `voiceroom`이며 `chatId`와 함께 지정할 수 있습니다. 단일 `/api/vox/audio` 요청은 최대 96,000바이트의 완전한 16-bit sample이어야 합니다.

## Iris API

`file`, `markdown`, `custom`은 Iris `POST /reply`로 전송합니다.

### Iris `POST /reply`

지원 타입은 `NOA_IRIS_HOOK_TYPES`로 제한됩니다.

Markdown:

```json
{"type":"markdown","room":"123456789","data":"**굵은 메시지**"}
```

파일(Android 경로 또는 Base64/Data URI):

```json
{"type":"file","room":"123456789","path":"/sdcard/Download/report.pdf"}
```

```json
{"type":"file","room":"123456789","data":"data:application/pdf;name=report.pdf;base64,..."}
```

Custom:

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

`data.type`: `1..65535`. `data.chat_id`는 `room`과 같아야 합니다.

`file`과 `markdown` 성공 응답:

```json
{"success":true,"message":"success"}
```

`custom` 성공 응답:

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

### Iris 확장 엔드포인트

기본 prefix는 `http://127.0.0.1:3000/noa`이며 `NOA_IRIS_ENDPOINT_PREFIX`로 변경합니다.

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
PCM 본문도 `application/octet-stream` 그대로 전송하며 Base64 변환은 내부 브리지에서만 수행합니다. Iris의 Ktor gateway는 요청 본문을 전달 전에 메모리에 모으므로 장시간 실시간 입력은 `/noa/vox/audio`에 작은 PCM 청크를 순차 전송하거나 Noa의 `/api/vox/audio/stream`을 직접 사용하십시오.

## 주요 환경변수

| 변수 | 기본값 | 설명 |
|---|---|---|
| `NOA_BIND` | `0.0.0.0:4000` | Noa HTTP 바인딩 주소 |
| `NOA_API_TOKEN` | 설치 시 자동 생성 | 공개 Noa API 인증 토큰 |
| `NOA_IRIS_HOOK` | `false` | Iris 네이티브 브리지 활성화 |
| `NOA_IRIS_HOOK_TYPES` | `file,markdown,custom` | Iris `/reply` 처리 타입 |
| `NOA_IRIS_ENDPOINT_PREFIX` | `/noa` | Iris 확장 API prefix |
| `KAKAO_HOOK_ENABLED` | `true` | Kakao 네이티브 작업 사용 여부 |
| `NOA_DATA_DIR` | `/data/local/tmp/noa` | 감사 DB와 설정 저장 경로 |
| `NOA_UPLOAD_DIR` | KakaoTalk 외부 파일 경로 | 임시 업로드 저장 경로 |
| `NOA_KAKAO_PATH` | 자동 탐색 | KakaoTalk 앱 데이터 경로 |
| `NOA_ANDROID_USER` | `0` | Android 사용자 ID |
| `NOA_MAX_UPLOAD_BYTES` | `67108864` | 요청 파일 최대 바이트 |
| `NOA_POLL_INTERVAL_MS` | `30000` | 이벤트 feed 안전 폴링 주기 |
| `NOA_SNAPSHOT_INTERVAL_MS` | `60000` | 방 스냅샷 안전 갱신 주기 |
| `NOA_SEND_INTERVAL_MS` | `300` | 전송 간격 |
| `NOA_IMAGE_MAX_DIMENSION` | `4096` | 이미지 최대 가로·세로 크기 |
| `NOA_JPEG_QUALITY` | `85` | JPEG 품질 (`50..95`) |
