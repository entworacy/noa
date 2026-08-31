# 1. 전체 구조와 실행 흐름

[README로 돌아가기](../README.md) · 다음: [데이터 관찰과 이벤트 처리](2-data-and-events.md)

## 목적과 경계

Noa는 Android 안에서 실행되는 단일 Rust HTTP 서비스입니다. KakaoTalk 앱을 대체하지 않고, 로그인된 KakaoTalk의 데이터와 Android 프레임워크 기능을 조합해 조회·전송·관리 API를 제공합니다. 플랫폼 내부 형식은 KakaoTalk 버전에 따라 바뀔 수 있으므로 외부 HTTP 계약과 내부 어댑터를 분리합니다.

## 구성 요소

| 구성 요소 | 위치 | 역할 |
|---|---|---|
| HTTP 서버 | `src/web` | 공개 API, 대시보드, Iris 내부 브리지 |
| Room catalog | `src/kakao` | KakaoTalk DB 열기, 방·멤버·프로필 조회, custom 행 관리 |
| Android relay | `src/device` | Intent 호출, 파일 URI 구성, 접근성 작업 직렬화 |
| Reconciler | `src/reconcile.rs` | DB 변경을 Room 캐시와 감사 이벤트로 반영 |
| Kakao agent | `kakao-agent` | KakaoTalk 프로세스 내부 명령 및 LOCO/Room 콜백 |
| Iris agent | `iris-agent` | Iris `/reply` 확장과 Noa gateway 연결 |
| UI agent | `ui-agent` | 상주 UiAutomator 의미 기반 탐색과 클릭 |

HTTP 서버는 외부 요청을 검증한 뒤 Room catalog 또는 Android relay로 전달합니다. 프로세스 내부 작업이 필요한 경우 `src/intercept`가 로컬 TCP 명령 채널을 통해 Kakao/Iris 에이전트와 통신합니다. 에이전트가 보내는 DB 무효화와 LOCO 이벤트는 별도 이벤트 채널로 들어옵니다.

## 시작 순서

1. `Settings::from_env`가 환경변수를 읽고 기본값과 하한·상한을 적용합니다.
2. 데이터 및 업로드 디렉터리를 만들고 Iris 브리지 설정을 권한 `0600`으로 게시합니다.
3. Noa 감사 DB를 열고 KakaoTalk DB 경로와 암호화 설정을 탐색합니다.
4. Android JNI relay를 준비하고 Room 캐시·SSE broadcast 채널을 생성합니다.
5. DB를 사용할 수 있으면 reconciler가 초기 스냅샷과 feed cursor를 읽습니다.
6. HTTP 서버를 시작한 뒤 설정에 따라 Kakao/Iris 에이전트를 주입합니다.
7. 업로드 임시 파일 정리와 선택적 `CHATONROOM` 순회를 백그라운드에서 실행합니다.

DB 또는 Android relay를 열지 못해도 서버는 제한 모드로 시작할 수 있습니다. 이 상태는 `/api/status`의 `databaseAvailable`, `androidAvailable`로 구분합니다. 설정의 활성 여부와 실제 에이전트 연결 여부도 각각 `Enabled`, `Active` 필드로 분리합니다.

## 동시성과 작업 분리

Actix worker는 HTTP 파싱과 응답을 담당합니다. SQLite처럼 블로킹될 수 있는 작업은 `spawn_blocking`으로 옮겨 async executor를 막지 않습니다. Room 목록은 `RwLock<Vec<Room>>` 캐시로 제공하고 감사 이벤트는 `broadcast` 채널을 사용해 SSE 구독자에게 전달합니다.

화면을 조작하는 입장·공유·강퇴·나가기 작업은 동일한 접근성 잠금을 공유합니다. 요청이 동시에 들어와도 두 작업이 KakaoTalk 화면을 서로 덮어쓰지 않습니다. Kakao 에이전트 명령은 ID별 상태와 timeout을 가지며, 중복 활성 ID는 기존 상태를 덮어쓰지 않고 거부합니다.

## 인증 경계

공개 Noa API는 `NOA_API_TOKEN`이 설정되었을 때 Bearer, `X-Noa-Token`, query token을 확인합니다. `/health`와 HTML 대시보드는 인증하지 않습니다. query token은 접근 로그와 브라우저 기록에 남을 수 있어 운영 환경에서는 헤더를 사용해야 합니다.

Iris 내부 브리지는 공개 토큰과 분리된 `NOA_IRIS_HOOK_TOKEN` 및 `X-Noa-Hook-Token`을 사용합니다. `/internal/iris/reply`와 `/internal/iris/endpoint`는 Iris 에이전트 전용이며 외부 클라이언트가 직접 호출하는 계약이 아닙니다. Endpoint 요청 본문은 Ktor에서 raw byte로 수신하고 내부 TCP 구간에서 Base64로 운반해 VOX PCM을 포함한 바이너리를 보존합니다.

## 전체 환경변수

| 변수 | 기본값 | 역할 |
|---|---|---|
| `NOA_BIND` | `0.0.0.0:4000` | HTTP 수신 주소 |
| `NOA_API_TOKEN` | 없음 | 공개 API 인증. 설치 스크립트는 자동 생성 |
| `NOA_DATA_DIR` | Android: `/data/local/tmp/noa` | 감사 DB와 설정 저장 위치 |
| `NOA_UPLOAD_DIR` | KakaoTalk 외부 파일 디렉터리 | 공유할 임시 파일 위치 |
| `NOA_KAKAO_PATH` | 자동 탐색 | KakaoTalk 앱 데이터 루트 |
| `NOA_ANDROID_USER` | `0` | Android 사용자 ID |
| `NOA_MAX_UPLOAD_BYTES` | `67108864` | 업로드 및 JSON payload 한도 기준 |
| `NOA_POLL_INTERVAL_MS` | `30000` | feed 안전 폴링. 최소 1000ms |
| `NOA_SNAPSHOT_INTERVAL_MS` | `60000` | Room 안전 갱신. 최소 5000ms |
| `NOA_SEND_INTERVAL_MS` | `300` | 전송 큐 간격 |
| `NOA_CALLING_PACKAGE` | `com.android.shell` | Intent 호출자로 사용할 패키지 |
| `NOA_FILE_PROVIDER_AUTHORITY` | 자동 탐색 | 공유 URI의 FileProvider authority |
| `NOA_IMAGE_MAX_DIMENSION` | `4096` | 이미지 리사이즈 기준. 최소 256 |
| `NOA_JPEG_QUALITY` | `85` | JPEG 품질. `50..95`로 제한 |
| `KAKAO_HOOK_ENABLED` | `true` | Kakao 내부 에이전트 경로 활성화 |
| `NOA_CHATONROOM_INTERVAL_MS` | `10000` | Room 순회 간격. `0`이면 비활성화 |
| `NOA_LOCO_HISTORY_LIMIT` | `1000` | 메모리 패킷 이력. `100..10000` |
| `NOA_IRIS_HOOK` | `false` | Iris 에이전트 활성화 |
| `NOA_IRIS_HOOK_TOKEN` | 자동 생성 | Iris 내부 브리지 인증 토큰 |
| `NOA_IRIS_HOOK_TYPES` | `file,markdown,custom` | 가로챌 Iris reply 타입 |
| `NOA_IRIS_HOOK_CONFIG` | `$NOA_DATA_DIR/iris-hook.json` | Iris 설정 게시 경로 |
| `NOA_IRIS_BRIDGE_URL` | 로컬 `/internal/iris/reply` | reply 내부 브리지 URL |
| `NOA_IRIS_ENDPOINT_PREFIX` | `/noa` | Iris 공개 확장 prefix |
| `NOA_IRIS_ENDPOINT_BRIDGE_URL` | 로컬 `/internal/iris/endpoint` | 확장 API 내부 브리지 URL |

## 빌드 구조

`scripts/build-android.sh`는 UI/Iris/Kakao Java 어댑터를 먼저 DEX로 만들고 ABI별 네이티브 에이전트와 Noa 바이너리를 빌드합니다. Kakao와 Iris 에이전트 `.so`는 최종 Noa 바이너리에 포함되며 실행 시 대상 프로세스로 주입됩니다. LSPlant, Frida, Android NDK 및 정적 C++ 런타임은 Android 네이티브 빌드에만 필요합니다.

개발 호스트에서는 다음 검증을 실행할 수 있습니다.

```bash
cargo test --locked
cargo test --manifest-path kakao-agent/Cargo.toml
cargo clippy --manifest-path kakao-agent/Cargo.toml --all-targets -- -D warnings
```
