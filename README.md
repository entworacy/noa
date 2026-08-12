# noa

루팅된 Android 또는 Redroid에서 KakaoTalk 파일·텍스트 전송과 채팅방 대시보드를 제공하는 바이너리입니다. ARM64, ARMv7, x86, x86_64 릴리스를 제공합니다. KakaoTalk 내부 DB와 비공개 API를 사용하므로 본인 소유 환경에서만 사용하십시오.

## 설치

필요 조건은 Android API 26 이상, root 권한, 로그인된 KakaoTalk, ADB 연결, Linux의 `bash`, `curl`, `python3`, `sha256sum`입니다.

```bash
git clone --depth 1 https://github.com/entworacy/noa.git
cd noa
adb connect 127.0.0.1:5555
./scripts/iris-noa install --serial 127.0.0.1:5555
./scripts/iris-noa up
```

Redroid를 새로 구성하는 경우에는 먼저 다음 명령을 실행합니다.

```bash
./scripts/iris-noa environment
```

`install`은 연결된 기기의 ABI를 확인하고 GitHub Releases에서 해당 Noa 바이너리와 `SHA256SUMS`를 내려받습니다. `/data/local/tmp/Iris.apk`가 이미 있으면 Iris를 다시 설치하지 않습니다. 설치가 끝나면 API 토큰과 설정을 `.noa-device.env`에 저장합니다.

주요 설치 옵션은 다음과 같습니다.

```text
--serial SERIAL
--release-version VERSION
--noa-binary PATH
--bind ADDRESS:PORT
--api-token TOKEN
--kakao-hook-enabled true|false
--hook-types file,markdown,custom
```

기본 주소는 `0.0.0.0:4000`, Iris 기본 포트는 `3000`입니다. 설치 후 상태 확인은 `./scripts/iris-noa status`, 중지는 `./scripts/iris-noa stop`으로 수행합니다.

## API

Base URL: `http://127.0.0.1:4000`

**`type`을 지정하는 `file`, `markdown`, `custom` 요청은 Noa의 `4000` 포트로 직접 호출하지 않습니다. 반드시 Iris의 `POST /reply` 엔드포인트로 요청해야 합니다.** 기본 Iris URL은 `http://127.0.0.1:3000/reply`이며, Noa는 허용된 타입의 요청만 내부에서 가로채 처리합니다.

인증이 활성화된 경우 일반 API에는 다음 헤더가 필요합니다.

```http
Authorization: Bearer API_TOKEN
```

오류 응답은 `{"error":"설명"}` 형식입니다.

### `GET /health`

인증 없이 서버 생존 여부를 확인합니다.

```json
{"ok":true}
```

### `GET /api/status`

서버, DB, Android, Iris·KakaoTalk 후킹 상태를 반환합니다.

```json
{
  "version":"1.2.1",
  "revision":"development",
  "databaseAvailable":true,
  "androidAvailable":true,
  "authenticationEnabled":true,
  "roomCount":3,
  "currentUserId":"123456789",
  "maxUploadBytes":67108864,
  "irisHookEnabled":true,
  "irisHookActive":true,
  "kakaoHookEnabled":true,
  "kakaoHookActive":true
}
```

### `GET /api/rooms`

참여 중인 채팅방과 참여자 목록을 반환합니다.

### `GET /api/rooms/{chatId}`

`chatId`가 일치하는 채팅방 하나를 반환합니다. 없으면 `404`입니다.

### `POST /send`

파일 하나를 전송합니다. `multipart/form-data` 또는 원시 바이너리를 사용할 수 있으며 최대 크기는 `NOA_MAX_UPLOAD_BYTES`입니다.

Multipart 필드:

| 이름 | 타입 | 필수 | 설명 |
|---|---|---:|---|
| `chatId` | string/integer | 예 | 대상 채팅방 ID |
| `file` | binary | 예 | 전송할 파일 |

원시 바이너리에서는 `chatId`를 query string으로 보내고 `filename` 또는 `X-Filename`으로 파일명을 지정합니다.

```bash
curl -X POST \
  -H 'Authorization: Bearer API_TOKEN' \
  -F 'chatId=CHAT_ID' -F 'file=@photo.png' \
  http://127.0.0.1:4000/send
```

응답:

```json
{"ok":true,"chatId":"CHAT_ID","file":{},"message":"KakaoTalk 공유 Intent를 실행했습니다"}
```

### `POST /send/text`

```json
{"chatId":"CHAT_ID","text":"안녕하세요","threadId":null}
```

`chatId`와 `threadId`는 문자열 또는 정수이며 `text`는 공백만으로 구성될 수 없습니다.

### `POST /api/open-chat/join`

오픈채팅 링크에 입장합니다.

```json
{"url":"https://open.kakao.com/o/OPEN_CHAT_ID","profile":"오픈프로필 이름"}
```

`profile`은 생략할 수 있으며, 생략 시 첫 번째 프로필을 사용합니다.

### `POST /api/rooms/{chatId}/leave`

지정한 채팅방에서 나갑니다. KakaoTalk 접근성 레이아웃에서 실제 버튼을 찾아 처리합니다.

### `POST /api/rooms/{chatId}/kick`

오픈채팅 참여자를 강퇴합니다.

```json
{"nickname":"닉네임"}
```

또는 다음처럼 `userId`를 사용할 수 있습니다.

```json
{"userId":"123456789"}
```

두 필드를 함께 보내면 같은 참여자를 가리켜야 하며 자기 자신은 강퇴할 수 없습니다.

### `GET /api/events`

입장, 퇴장, 닉네임 변경 감사 로그를 반환합니다.

| Query | 타입 | 설명 |
|---|---|---|
| `chatId` | integer | 특정 방만 조회 |
| `userId` | integer | 특정 사용자 조회. `chatId`와 함께 사용 |
| `limit` | integer | 반환 개수. 기본 200 |

예: `/api/events?chatId=CHAT_ID&userId=123456789&limit=50`

### `GET /api/events/stream`

감사 로그 변경을 이벤트 스트림으로 수신합니다. 일반 API 인증 헤더를 사용합니다.

### Iris `POST /reply`

`type` 기반 전송의 공개 호출 지점은 Iris입니다. `http://127.0.0.1:4000/internal/iris/reply`는 Iris 후킹 에이전트와 Noa 사이의 내부 브리지이므로 직접 호출하지 마십시오. Iris의 `/reply`로 들어온 요청 중 `NOA_IRIS_HOOK_TYPES`에 허용된 타입만 Noa가 가로채 처리합니다.

```bash
curl -X POST \
  -H 'Content-Type: application/json' \
  --data '{"type":"markdown","room":"CHAT_ID","data":"**굵은 메시지**"}' \
  http://127.0.0.1:3000/reply
```

공통 본문:

```json
{"type":"file|markdown|custom","room":"CHAT_ID","data":{}}
```

`file`은 Android 절대 경로 또는 Base64 데이터로 보냅니다. `path`와 `data`가 모두 있으면 `data`가 우선합니다.

```json
{"type":"file","room":"CHAT_ID","path":"/sdcard/Download/report.pdf"}
{"type":"file","room":"CHAT_ID","data":"data:application/pdf;name=report.pdf;base64,..."}
```

`markdown`은 `data`에 Markdown 텍스트를 넣습니다.

```json
{"type":"markdown","room":"CHAT_ID","data":"**굵은 메시지**"}
```

`custom` 본문:

```json
{
  "type":"custom",
  "room":"CHAT_ID",
  "data":{
    "type":1,
    "message":"메시지",
    "attachment":{},
    "supplement":null,
    "chat_id":"CHAT_ID",
    "thread_id":null,
    "scope":1,
    "v":null,
    "is_silence":0
  }
}
```

`custom`은 `chat_sending_logs`에 행을 등록한 뒤 `KAKAO_HOOK_ENABLED` 설정에 따라 발신합니다. `true`이면 Rust 에이전트가 KakaoTalk 내부 재전송 함수를 호출하고, `false`이면 KakaoTalk 재실행·채팅방 인텐트·접근성 레이아웃 경로를 사용합니다. `attachment`, `supplement`, `v`는 JSON 객체 또는 JSON 문자열입니다.

위의 `file`, `markdown`, `custom` 예시는 모두 Iris `POST /reply`의 요청 본문입니다. Noa `POST /send`와 `POST /send/text`에는 `type`을 넣지 않습니다.

## 환경변수

설치 스크립트가 저장하는 값은 `.noa-device.env`에 기록됩니다. 실행 시 환경변수로 덮어쓸 수 있습니다.

| 변수 | 기본값 | 설명 |
|---|---|---|
| `NOA_BIND` | `0.0.0.0:4000` | HTTP 바인딩 주소 |
| `NOA_API_TOKEN` | 자동 생성 | 일반 API Bearer 토큰 |
| `NOA_IRIS_HOOK` | `false` | Iris 네이티브 후킹 활성화 |
| `NOA_IRIS_HOOK_TOKEN` | 자동 생성 | Iris 내부 브리지 토큰 |
| `NOA_IRIS_HOOK_TYPES` | `file,markdown,custom` | Iris에서 가로챌 타입 |
| `NOA_IRIS_HOOK_CONFIG` | `$NOA_DATA_DIR/iris-hook.json` | Iris 설정 파일 |
| `NOA_IRIS_BRIDGE_URL` | 현재 Noa 포트의 `/internal/iris/reply` | Iris 브리지 URL |
| `KAKAO_HOOK_ENABLED` | `true` | custom 발신 방식 선택 |
| `NOA_CHATONROOM_INTERVAL_MS` | `10000` | 보유한 오픈채팅방을 순회하며 내부 CHATONROOM 함수를 호출하는 간격. `0`이면 비활성화 |
| `NOA_DATA_DIR` | Android `/data/local/tmp/noa` | DB·로그·설정 디렉터리 |
| `NOA_UPLOAD_DIR` | KakaoTalk 전용 업로드 경로 | 파일 임시 저장 경로 |
| `NOA_KAKAO_PATH` | 자동 탐색 | KakaoTalk DB 경로 |
| `NOA_ANDROID_USER` | `0` | Android 사용자 ID |
| `NOA_MAX_UPLOAD_BYTES` | `67108864` | 업로드 최대 바이트 |
| `NOA_POLL_INTERVAL_MS` | `250` | DB 폴링 간격. 최소 50 |
| `NOA_SNAPSHOT_INTERVAL_MS` | `3000` | 방 목록 갱신 간격. 최소 500 |
| `NOA_SEND_INTERVAL_MS` | `300` | 전송 간격 |
| `NOA_CALLING_PACKAGE` | `com.android.shell` | Android Intent 호출 패키지 |
| `NOA_FILE_PROVIDER_AUTHORITY` | 자동 탐색 | 파일 공유 Provider authority |
| `NOA_IMAGE_MAX_DIMENSION` | `4096` | 이미지 최대 변 |
| `NOA_JPEG_QUALITY` | `85` | JPEG 품질. 50~95 |

`KAKAO_HOOK_ENABLED=false`는 `custom` 발신만 좌표 기반 경로로 바꿉니다. 인증 토큰은 로그나 공개 URL에 노출하지 마십시오.
