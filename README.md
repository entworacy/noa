# noa

<p align="center">
  <img src="assets/noa-cover.jpg" alt="Noa KakaoTalk illustration" width="480">
</p>

Noa는 루팅된 Android 또는 Redroid에서 KakaoTalk 채팅방을 조회하고 메시지·파일 전송, 오픈채팅 관리, 참여자 변경 감사를 제공하는 Android 네이티브 서비스입니다. ARM64, ARMv7, x86, x86_64를 지원합니다.

> **주의:** Noa는 KakaoTalk 내부 데이터베이스, 비공개 API와 프로세스 후킹을 사용하며 KakaoTalk 운영정책 또는 이용약관에 위배될 수 있습니다. 사용 과정에서 계정 정지, 서비스 이용 제한, 데이터 손상이나 기타 불이익이 발생할 수 있으므로 본인이 소유하고 관리할 권한이 있는 테스트 환경에서만 사용하십시오. 관련 정책과 법령을 확인하고 준수할 책임 및 사용으로 발생하는 결과에 대한 책임은 사용자에게 있습니다. 이 프로젝트는 Kakao 및 KakaoTalk과 제휴하거나 공식 승인을 받은 제품이 아닙니다.

## 법적 고지 및 책임 제한

Noa는 연구·개발 및 상호운용성 검증 목적으로, 어떠한 명시적·묵시적 보증 없이 **“있는 그대로(AS IS)”** 제공됩니다. 개발자와 기여자는 정확성, 안정성, 지속적인 동작, 특정 목적 적합성, 상품성, 비침해성 또는 KakaoTalk의 특정 버전·환경과의 호환성을 보증하지 않습니다.

사용자는 Noa를 설치·실행하기 전에 적용되는 법령, KakaoTalk 이용약관·운영정책 및 제3자의 권리를 직접 확인하고, 필요한 권한과 동의를 확보해야 합니다. 계정 제재, 서비스 이용 제한, 기기 또는 데이터 손상·유실, 보안 사고, 개인정보 노출, 영업 중단 및 기타 직접·간접·특별·부수·결과적 손해를 포함하여 사용 또는 사용 불능으로 발생하는 위험은 사용자가 부담합니다.

관련 법률이 허용하는 최대 범위에서 개발자와 기여자는 위 손해에 대해 책임을 부담하지 않습니다. 다만 적용 법률상 제한하거나 배제할 수 없는 책임에는 이 문구가 적용되지 않습니다. 이 문서는 법률 자문이 아니며, 구체적인 적법성이나 책임 범위가 중요한 경우 관할 지역의 자격 있는 전문가에게 문의하십시오. 라이선스의 정식 무보증 및 책임 제한 조건은 [Apache License 2.0](LICENSE)의 제7조와 제8조를 따릅니다.

## 지원 기준 및 후킹 안내

| 항목 | 내용 |
|---|---|
| 최적화·검증 기준 | KakaoTalk Android `26.6.3` |
| 실행 환경 | Android API 26 이상, root 권한 필요 |
| 기본 동작 | `KAKAO_HOOK_ENABLED=true` |
| 후킹 방식 | Frida Core로 KakaoTalk·Iris 프로세스에 네이티브 에이전트를 주입하고 LSPlant로 필요한 Java 메서드를 후킹 |
| 비후킹 모드 | `KAKAO_HOOK_ENABLED=false`로 전환하며 지원 작업은 Android Intent와 의미 기반 UiAutomator로 처리 |

Noa는 KakaoTalk `26.6.3`의 난독화된 클래스·메서드, 데이터베이스 스키마와 UI resource ID를 기준으로 최적화되어 있습니다. 다른 버전에서도 일부 기능은 동작할 수 있지만 호환성을 보장하지 않으며, KakaoTalk 업데이트 후에는 custom 전송, 오픈채팅 입장·프로필 공유·강퇴와 이벤트 감지를 실제 기기에서 다시 검증해야 합니다.

후킹은 custom 발신과 일부 오픈채팅 관리 작업, Room 변경 및 LOCO 관찰에 사용됩니다. 일반 텍스트·파일·Markdown 전송은 후킹 설정과 관계없이 Android Intent를 사용합니다. 후킹을 끄면 가능한 관리 작업은 접근성 경로로 전환되지만, 내부 관찰 기능 일부는 비활성화될 수 있습니다. 자세한 내용은 [후킹 및 접근성 에이전트](docs/4-agents.md)를 참고하십시오.

## 동작 원리

Noa는 KakaoTalk 데이터베이스를 읽어 채팅방과 참여자 상태를 구성하고, Android Intent로 일반 메시지와 파일을 전송합니다. custom 메시지와 일부 관리 작업은 설정에 따라 KakaoTalk 프로세스의 네이티브 에이전트 또는 의미 기반 UiAutomator 경로를 사용합니다. 작업 성공은 가능한 경우 UI 클릭이나 함수 반환만으로 판단하지 않고 KakaoTalk 데이터베이스의 사후 상태로 다시 검증합니다.

자세한 구현 원리는 다음 문서에 나누어 정리되어 있습니다.

1. [전체 구조와 실행 흐름](docs/1-architecture.md)
2. [데이터 관찰과 이벤트 처리](docs/2-data-and-events.md)
3. [메시지·파일 전송 원리](docs/3-delivery.md)
4. [후킹 및 접근성 에이전트](docs/4-agents.md)

## 설치

이 문서는 루팅된 Android 또는 Redroid에 Iris와 로그인된 KakaoTalk이 이미 설치되어 있다는 전제로 설명합니다. Iris 자체의 설치와 Redroid 환경 구성은 Iris 프로젝트의 안내를 따르십시오.

추가로 필요한 항목:

- Android API 26 이상
- ADB로 연결할 수 있는 Linux 환경
- `bash`, `curl`, `python3`, `sha256sum`

먼저 Iris가 설치된 Android에 ADB로 연결한 뒤 Noa를 설치합니다.

```bash
git clone --depth 1 https://github.com/entworacy/noa.git
cd noa
adb connect 127.0.0.1:5555
./scripts/iris-noa install --serial 127.0.0.1:5555
./scripts/iris-noa up
```

다른 ADB 주소를 사용한다면 `127.0.0.1:5555`를 실제 serial로 바꾸십시오. 로컬에 별도로 받은 `iris_control`이 있다면 설치 명령에 `--iris-control PATH`를 추가할 수 있습니다.

설치 스크립트는 `/data/local/tmp/Iris.apk`의 기존 Iris 설치를 유지하면서 기기 ABI에 맞는 Noa 릴리스와 체크섬을 내려받습니다. Noa API 토큰과 실행 설정은 권한이 제한된 `.noa-device.env`에 저장됩니다. 설치 완료 후 출력되는 API 토큰은 Noa의 4000 포트에 직접 요청할 때 사용합니다.

```bash
./scripts/iris-noa status
./scripts/iris-noa stop
```

기본 Noa 주소는 `http://127.0.0.1:4000`, 기본 Iris 주소는 `http://127.0.0.1:3000`입니다. 실제 ADB 포워딩 포트는 `status` 출력에서 확인하십시오.

## Python 클라이언트

Python에서 Noa 기능을 사용할 때는 [`irispy-noa-client`](https://github.com/entworacy/irispy-noa-client) 사용을 권장합니다. 기존 `irispy-client`와 같은 `iris` import를 유지하면서 Noa의 Iris 확장 엔드포인트와 Markdown, custom 메시지, 멘션 편의 기능을 사용할 수 있습니다. Python 3.10 이상이 필요합니다.

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

Iris의 3000 포트에 있는 `/noa/...` API를 사용할 때 Noa API 토큰을 Python 코드에 직접 넣을 필요는 없습니다. 주입된 Iris 에이전트가 내부 브리지 토큰을 자동으로 첨부합니다. 패키지를 교체한 뒤에는 실행 중인 봇을 다시 시작하십시오.

## API 공통 규칙

Noa API의 기본 URL은 `http://127.0.0.1:4000`입니다. `/health`, `/`, `/dashboard`, `/loco`를 제외한 공개 Noa API에는 인증이 적용됩니다. `NOA_API_TOKEN`이 설정된 경우 다음 중 하나를 사용할 수 있습니다.

```http
Authorization: Bearer API_TOKEN
X-Noa-Token: API_TOKEN
```

`?token=API_TOKEN`도 지원하지만 URL과 로그에 토큰이 남을 수 있으므로 헤더 사용을 권장합니다. JSON 요청에는 `Content-Type: application/json`을 지정합니다.

성공 응답은 엔드포인트별 JSON 또는 SSE이고, 오류 응답은 공통적으로 다음 형식입니다.

```json
{"error":"오류 설명"}
```

| 상태 | 의미 |
|---:|---|
| `400` | 요청 형식 또는 값 오류 |
| `401` | 인증 실패 |
| `404` | 대상 또는 기능을 찾을 수 없음 |
| `503` | Android/에이전트 기능을 사용할 수 없음 |
| `500` | 데이터베이스 또는 내부 처리 오류 |

`chatId`, `userId`, `profileId`, `linkId`는 큰 정수의 JSON 정밀도 손실을 피하기 위해 응답에서 문자열로 반환합니다. 요청에서 허용되는 타입은 각 항목에 별도로 표시합니다.

## 상태 및 조회 API

### `GET /health`

인증 없이 프로세스 생존 여부를 확인합니다.

```json
{"ok":true,"service":"noa","version":"1.3.3"}
```

### `GET /api/status`

서버와 KakaoTalk 연동 상태를 반환합니다.

```json
{
  "version":"1.3.3",
  "revision":"development",
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

`currentUserId`는 DB가 없으면 `null`입니다. `Enabled`는 설정 여부, `Active`는 에이전트 연결 여부를 뜻합니다.

### `GET /api/rooms`

참여 중인 채팅방 배열을 반환합니다.

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

동일한 Room 객체 하나를 반환합니다. 방이 없으면 `404`입니다.

### `GET /api/events`

저장된 참여자 이벤트 배열을 최신순으로 반환합니다.

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

`Content-Type: text/event-stream`으로 새 RoomEvent를 전달합니다.

```text
: connected

data: {"id":2,"chatId":"123456789","roomName":"테스트 방","kind":"left","userId":"111","nickname":"사용자","previousNickname":null,"occurredAt":1720000010,"source":"snapshot"}

```

### `GET /api/loco?limit=500`

최근 LOCO 패킷 배열을 최신순으로 반환합니다. `limit` 기본값은 `500`, 허용 범위는 `1..10000`입니다. `/loco`는 인증 없는 진단용 HTML 화면입니다.

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

파일 하나를 전송합니다. 최대 크기는 `NOA_MAX_UPLOAD_BYTES`입니다.

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

`chatId`와 `threadId`는 JSON 문자열 또는 정수입니다. `threadId`는 생략하거나 `null`로 지정할 수 있고, `text`는 공백만으로 구성될 수 없습니다.

```json
{"chatId":"123456789","text":"안녕하세요","threadId":null}
```

```json
{"ok":true,"chatId":"123456789","message":"KakaoTalk 답장 Intent를 실행했습니다"}
```

## 오픈채팅 및 방 관리 API

### `GET /api/open-chat/profiles`

입장에 사용할 수 있는 소유 프로필을 반환합니다.

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

소유 오픈프로필의 검증된 공유 URL을 반환합니다. `linkId`는 양의 64비트 정수 문자열만 허용합니다. `mode`는 `auto`, `hook`, `accessibility`이며 기본값은 `auto`입니다.

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

방 참여자의 오픈프로필 공유 URL을 반환합니다. `chatId`는 문자열, `userId`는 문자열 또는 정수입니다.

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

멤버의 `linkId`를 확인할 수 있으면 `/profiles/share`와 같은 응답 형식이 반환될 수 있습니다.

### `POST /api/open-chat/join`

정규형 `https://open.kakao.com/o/...` URL로 오픈채팅에 입장합니다. `profileId`는 소유 프로필 목록의 ID이며 생략하거나 `null`이면 목록의 첫 프로필을 사용합니다. 알 수 없는 필드는 허용하지 않습니다.

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

이미 참여 중인 방이면 `profileId`와 `profile`은 `null`, `profileApplied`는 `false`입니다.

### `POST /api/rooms/{chatId}/kick`

`nickname` 또는 `userId`로 참여자를 강퇴합니다. `userId`는 문자열 또는 정수입니다. 둘을 함께 보내면 같은 참여자를 가리켜야 하며 자기 자신은 대상이 될 수 없습니다.

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

요청 본문 없이 지정한 방에서 나갑니다.

```json
{
  "ok":true,
  "chatId":"123456789",
  "roomName":"오픈채팅방",
  "message":"채팅방 나가기를 완료했습니다"
}
```

## Iris API

`file`, `markdown`, `custom` 타입 요청은 Noa의 4000 포트가 아니라 Iris의 `POST /reply`로 전송합니다. Noa의 `/internal/iris/...` 경로는 에이전트 전용이며 공개 API가 아닙니다.

### Iris `POST /reply`

`room`은 문자열 또는 정수입니다. 처리할 수 있는 타입은 `NOA_IRIS_HOOK_TYPES`로 제한됩니다.

Markdown 요청:

```json
{"type":"markdown","room":"123456789","data":"**굵은 메시지**"}
```

파일 요청은 Android 절대 경로 또는 Base64/Data URI를 사용합니다. `data`와 `path`가 모두 있으면 `data`가 우선합니다.

```json
{"type":"file","room":"123456789","path":"/sdcard/Download/report.pdf"}
```

```json
{"type":"file","room":"123456789","data":"data:application/pdf;name=report.pdf;base64,..."}
```

Custom 요청:

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

`data.type`은 `1..65535`, `scope`와 `is_silence`는 0 이상이어야 합니다. `attachment`, `supplement`, `v`는 JSON 값 또는 유효한 JSON 문자열입니다. `data.chat_id`를 지정하면 바깥쪽 `room`과 같아야 합니다.

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

Iris 에이전트가 활성화되면 기본적으로 `http://127.0.0.1:3000/noa` 아래에 다음 엔드포인트가 노출됩니다. prefix는 `NOA_IRIS_ENDPOINT_PREFIX`로 변경할 수 있습니다.

| 메서드 | 경로 | Noa API 대응 |
|---|---|---|
| `GET` | `/noa/health` | 확장 게이트웨이 상태 |
| `GET` | `/noa/open-chat/profiles` | `GET /api/open-chat/profiles` |
| `POST` | `/noa/open-chat/profiles/share` | `POST /api/open-chat/profiles/share` |
| `POST` | `/noa/open-chat/profiles/share-member` | `POST /api/open-chat/profiles/share-member` |
| `POST` | `/noa/open-chat/join` | `POST /api/open-chat/join` |
| `POST` | `/noa/rooms/{chatId}/kick` | `POST /api/rooms/{chatId}/kick` |
| `POST` | `/noa/rooms/{chatId}/leave` | `POST /api/rooms/{chatId}/leave` |

요청·응답 본문은 대응하는 Noa API와 동일합니다. Iris가 내부 인증 헤더를 추가하므로 외부 호출자는 Noa API 토큰을 보내지 않습니다.

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

전체 실행 흐름과 나머지 고급 설정은 [전체 구조와 실행 흐름](docs/1-architecture.md)을 참고하십시오.

## 라이선스

Apache-2.0. 제3자 구성 요소와 상호운용 참고 사항은 [NOTICE](NOTICE)와 [licenses](licenses)를 확인하십시오.
