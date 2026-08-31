# 3. 메시지·파일 전송 원리

이전: [데이터 관찰과 이벤트 처리](2-data-and-events.md) · [README로 돌아가기](../README.md) · 다음: [후킹 및 접근성 에이전트](4-agents.md)

## 전송 경로 선택

Noa에는 두 종류의 전송 경로가 있습니다.

| 요청 | 기본 실행 경로 | 성공 판단 |
|---|---|---|
| `POST /send` | Android 공유 Intent | Intent 실행 완료 |
| `POST /send/text` | KakaoTalk 답장 Service Intent | Service 호출 완료 |
| Iris `markdown` | Android Intent | Intent 실행 완료 |
| Iris `file` | 파일 준비 후 공유 Intent | Intent 실행 완료 |
| Iris `custom` | Kakao 내부 재전송 또는 접근성 재전송 | DB 이동 확인 |

일반 텍스트·파일·Markdown은 `KAKAO_HOOK_ENABLED`와 무관하게 Android 프레임워크 Intent를 사용합니다. custom만 KakaoTalk 내부 DB 모델과 재전송 동작이 필요합니다.

## 파일 수신과 준비

`POST /send`는 multipart 파일 하나 또는 원시 request body를 받습니다. Iris 파일 요청은 Android 절대 경로와 Base64/Data URI를 지원합니다. 모든 경로는 디코딩 전 예상 크기와 디코딩 후 실제 크기를 `NOA_MAX_UPLOAD_BYTES`에 대조하고 빈 파일을 거부합니다.

파일 종류는 가능한 경우 실제 바이트 signature로 판별합니다. 선언된 Content-Type과 확장자는 보조 정보로만 사용합니다. 파일명에서 안전하지 않은 문자를 정리하고 판별된 MIME에 맞는 확장자를 선택한 뒤 요청별 UUID 디렉터리에 저장합니다.

JPEG, PNG, WebP가 다음 조건 중 하나에 해당하면 최적화를 시도합니다.

- 가로 또는 세로가 `NOA_IMAGE_MAX_DIMENSION`을 초과
- 원본이 5MiB를 초과

알파 채널이 있으면 PNG, 없으면 설정된 품질의 JPEG로 인코딩합니다. 축소가 필요하지 않은데 결과가 원본보다 크면 원본을 보존합니다. 응답의 `originalBytes`, `storedBytes`, `optimized`로 처리 결과를 확인할 수 있습니다.

## FileProvider URI

KakaoTalk이 파일을 읽을 수 있도록 임시 디렉터리와 파일 권한을 설정하고 `content://` URI를 만듭니다. KakaoTalk 전용 외부 파일 경로면 `com.kakao.talk.FileProvider` 규칙을 사용하고, 그 밖의 경우 설정 또는 자동 탐색된 authority를 사용합니다. Intent에는 읽기 권한 flag와 실제 MIME type을 포함합니다.

임시 파일은 전송 직후 즉시 삭제하지 않습니다. KakaoTalk이 비동기로 읽을 시간을 확보한 뒤 주기적 reaper가 오래된 작업 디렉터리를 정리합니다.

## 텍스트와 Markdown

`POST /send/text`는 KakaoTalk NotificationActionService의 reply Intent를 구성합니다. `threadId`를 제공하면 thread reply 문맥을 포함하고, 없으면 일반 텍스트로 전송합니다. Android 버전별 Binder signature는 시작 시 확인한 뒤 API 26–29와 API 30 이상의 정확한 메서드를 선택합니다.

Iris `markdown`은 Iris `/reply` 요청을 에이전트가 가로채 Noa 내부 브리지로 넘깁니다. Noa가 대상 Room과 비어 있지 않은 Markdown 문자열을 검증한 후 Android relay가 KakaoTalk 공유 경로로 전달합니다.

## Custom 메시지

Custom 전송은 임의의 KakaoTalk 메시지 모델을 직접 보내는 것이 아니라 DB 대기 행과 KakaoTalk의 정상 재전송 동작을 연결합니다.

1. 바깥쪽 `room`과 선택적 `data.chat_id`가 같은지 확인합니다.
2. 메시지 type, scope, JSON 열, 크기를 검증합니다.
3. `chat_sending_logs`에 새 행과 `client_message_id`를 기록합니다.
4. 후킹 모드에서는 Kakao 내부 sending manager를 호출합니다.
5. 접근성 모드에서는 대상 방 Activity와 방 제목을 확인하고 실패 표시가 있는 정확한 버블의 재전송 메뉴를 실행합니다.
6. 최대 약 6초 동안 같은 `client_message_id`가 실제 `chat_logs`로 이동했는지 확인합니다.

`attachment`, `supplement`, `v`는 JSON 객체/배열 등 값 자체 또는 JSON 문자열을 받을 수 있습니다. 문자열이면 저장 전에 JSON으로 다시 파싱해 유효성을 확인합니다. `clientMessageId`를 응답하는 이유는 호출자가 감사 로그와 KakaoTalk DB 결과를 연결할 수 있게 하기 위해서입니다.

## 전송 직렬화와 오류 의미

Android relay는 전송 큐를 두어 요청 사이에 `NOA_SEND_INTERVAL_MS` 간격을 적용합니다. 파일 staging처럼 CPU 또는 파일 I/O가 필요한 작업은 async worker를 막지 않도록 별도 작업으로 처리합니다.

Intent 기반 응답의 성공은 Android가 호출을 수락했다는 의미입니다. 반면 custom 응답의 `verified: true`, `verification: "database"`는 DB 사후 조건까지 확인했다는 뜻입니다. custom timeout 오류 뒤에는 이미 전송됐지만 관찰이 늦었을 가능성이 있으므로 즉시 동일 요청을 반복하기 전에 실제 방 상태를 확인해야 합니다.
