# Noa

<p align="center">
  <img src="assets/noa-cover.jpg" alt="Noa 프로젝트 커버" width="480">
</p>

**루팅된 Android에서 실행하는 카카오톡 관리 서비스**

Noa는 로그인된 카카오톡의 채팅방·참여자 조회, 메시지 전송, 오픈채팅 관리 기능을 HTTP API와 웹 대시보드로 제공합니다. Android 기기와 Redroid에서 실행하며, Iris를 통한 봇 연동도 지원합니다.

[빠른 시작](#빠른-시작) · [API 레퍼런스](docs/6-api.md) · [문서](#문서) · [라이선스](#라이선스)

## 주요 기능

- **조회 및 이벤트**: 채팅방·참여자·프로필 조회, 입장·퇴장·강퇴·닉네임 변경 기록, SSE 구독
- **메시지 전송**: 텍스트·이미지·파일 전송, Markdown → RichText 변환, custom 메시지 재전송
- **오픈채팅 관리**: 입장·나가기, 참여자 강퇴·강퇴 해제, 메시지 리액션·가리기, 프로필 공유 URL 조회
- **LOCO**: 송수신 패킷 조회와 기존 앱 연결을 통한 직접 발신
- **VOX**: 보이스톡·보이스룸 제어, PCM 오디오 송출
- **Iris 연동**: `/reply` 확장, `/noa` API 게이트웨이, Python 클라이언트 지원

## 지원 환경

카카오톡 Android `26.9.1`에 최적화되어 있습니다.

| 항목 | 지원 범위 |
| --- | --- |
| 운영체제 | Android API 26 이상, root 권한 필요 |
| 실행 환경 | Android 기기 또는 Redroid |
| 아키텍처 | ARM64, ARMv7, x86, x86_64 |
| 기본 실행 모드 | Frida Core + LSPlant 후킹 (`KAKAO_HOOK_ENABLED=true`) |
| 비후킹 모드 | Android Intent + UiAutomator를 통한 일부 기능 지원 |

Noa는 카카오의 공식 제품이 아닙니다. 카카오톡 내부 DB와 비공개 API를 사용하므로 앱 업데이트에 따라 호환성 수정이 필요할 수 있습니다. 기능별 실행 경로는 [에이전트 문서](docs/4-agents.md)에 정리되어 있습니다.

## 빠른 시작

### 준비 사항

- root 권한을 사용할 수 있고 카카오톡에 로그인된 Android 기기 또는 Redroid
- 기기에 ADB로 연결할 수 있는 Linux 호스트
- `git`, `adb`, `bash`, `curl`, `python3`, `sha256sum` 및 기본 Unix 도구

### 설치 및 실행

```bash
git clone --depth 1 https://github.com/entworacy/noa.git
cd noa

adb connect 127.0.0.1:5555
./scripts/iris-noa install --serial 127.0.0.1:5555
./scripts/iris-noa up
```

`127.0.0.1:5555`는 예시 ADB 주소입니다. 실제 기기 주소 또는 ADB 시리얼로 변경합니다.

설치 스크립트는 공개된 최신 릴리스에서 기기 아키텍처에 맞는 Noa 바이너리와 체크섬을 내려받습니다. Iris가 없으면 함께 설치하고, 기존 Iris 설치가 있으면 사용합니다. 설치 설정과 자동 생성된 API 토큰은 `.noa-device.env`에 저장됩니다.

필요에 따라 설치 옵션을 지정할 수 있습니다.

| 옵션 | 용도 |
| --- | --- |
| `--release-version VERSION` | 설치할 공개 릴리스 버전 지정 |
| `--noa-binary PATH` | 로컬 실행 파일 설치 |
| `--iris-control PATH` | 기존 `iris_control` 스크립트 사용 |
| `--api-token TOKEN` | 자동 생성 대신 지정한 API 토큰 사용 |
| `--kakao-hook-enabled false` | 비후킹 모드 사용 |

`--release-version`과 `--noa-binary`는 함께 사용할 수 없습니다. 전체 옵션은 `./scripts/iris-noa --help`로 확인합니다.

### 접속 및 상태 확인

기본 설정에서 다음 주소로 접속합니다.

| 서비스 | 주소 |
| --- | --- |
| Noa 대시보드 | `http://127.0.0.1:4000/` |
| LOCO 패킷 뷰어 | `http://127.0.0.1:4000/loco` |
| Noa HTTP API | `http://127.0.0.1:4000` |
| Iris | `http://127.0.0.1:3000` |

```bash
curl http://127.0.0.1:4000/health

./scripts/iris-noa status
./scripts/iris-noa stop
```

## 사용 예시

Noa API는 설치 시 생성된 토큰으로 인증합니다. 아래 `API_TOKEN`을 실제 토큰으로 바꿉니다.

채팅방 목록 조회:

```bash
curl http://127.0.0.1:4000/api/rooms \
  -H 'Authorization: Bearer API_TOKEN'
```

텍스트 메시지 전송:

```bash
curl -X POST http://127.0.0.1:4000/send/text \
  -H 'Authorization: Bearer API_TOKEN' \
  -H 'Content-Type: application/json' \
  -d '{"chatId":"123456789","text":"안녕하세요"}'
```

`chatId`는 조회한 채팅방 ID로 지정합니다. 텍스트·파일 전송의 성공 응답은 Android Intent 실행 완료를 나타냅니다. 실제 메시지 수신 여부와는 구분됩니다.

API별 요청 형식, 완료 기준, 오류 응답은 [API 레퍼런스](docs/6-api.md)에 정리되어 있습니다.

### Python 클라이언트

Python 3.10 이상에서는 [irispy-noa-client](https://github.com/entworacy/irispy-noa-client)를 사용할 수 있습니다. 기존 `irispy-client`가 설치되어 있다면 먼저 제거합니다.

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

Iris의 `/noa/...` 요청에는 에이전트가 내부 인증을 추가합니다. 지원 엔드포인트는 [Iris API](docs/6-api.md#iris-api)를 참고합니다.

## 설정

다음은 Android에서 Noa 서버를 실행할 때 사용하는 주요 환경변수입니다. 설치 스크립트의 `up` 명령은 `.noa-device.env`의 저장된 설정을 사용합니다.

| 변수 | 기본값 | 설명 |
| --- | --- | --- |
| `NOA_BIND` | `0.0.0.0:4000` | HTTP 수신 주소 |
| `NOA_API_TOKEN` | 없음 · 설치 스크립트에서 자동 생성 | 공개 API 인증 토큰 |
| `NOA_IRIS_HOOK` | `false` · `iris-noa up`에서 활성화 | Iris 네이티브 브리지 사용 |
| `NOA_IRIS_HOOK_TYPES` | `file,markdown,custom` | Iris `/reply` 확장 타입 |
| `NOA_IRIS_ENDPOINT_PREFIX` | `/noa` | Iris 확장 API 경로 접두사 |
| `KAKAO_HOOK_ENABLED` | `true` | 카카오톡 네이티브 작업 사용 |
| `NOA_DATA_DIR` | `/data/local/tmp/noa` | 감사 DB 및 설정 저장 경로 |
| `NOA_UPLOAD_DIR` | `/sdcard/Android/data/com.kakao.talk/files/noa/uploads` | 임시 업로드 경로 |
| `NOA_KAKAO_PATH` | 자동 탐색 | 카카오톡 앱 데이터 경로 |
| `NOA_ANDROID_USER` | `0` | Android 사용자 ID |
| `NOA_MAX_UPLOAD_BYTES` | `67108864` | 업로드 최대 크기 · 바이트 |
| `NOA_POLL_INTERVAL_MS` | `30000` | 이벤트 보완 폴링 주기 · ms |
| `NOA_SNAPSHOT_INTERVAL_MS` | `60000` | 채팅방 스냅샷 갱신 주기 · ms |
| `NOA_SEND_INTERVAL_MS` | `300` | 전송 큐의 처리 간격 · ms |
| `NOA_IMAGE_MAX_DIMENSION` | `4096` | 이미지 최대 가로·세로 크기 · px |
| `NOA_JPEG_QUALITY` | `85` | JPEG 품질 · `50..95` |

서버를 직접 실행할 때 `NOA_API_TOKEN`을 지정하지 않으면 공개 API 인증이 비활성화됩니다. 전체 설정과 기본값은 [환경변수 목록](docs/1-architecture.md#전체-환경변수)을 참고합니다.

## 문제 해결

| 증상 | 확인할 사항 |
| --- | --- |
| API가 `401`을 반환함 | 설치 시 발급된 토큰과 `Authorization` 헤더 확인 |
| 릴리스 다운로드 실패 | 공개 릴리스의 바이너리·체크섬 존재 여부 확인 |
| `databaseAvailable: false` | root 권한, 카카오톡 로그인 상태, 앱 데이터 경로 확인 |
| `androidAvailable: false` | `/data/local/tmp/noa.log`의 ART 사전 검사 및 JNI 초기화 로그 확인 |
| `kakaoHookActive: false` | 후킹 설정과 대상 카카오톡 버전, 에이전트 초기화 로그 확인 |
| LOCO client를 찾지 못함 | 후킹 이후 앱에서 정상 송신이 발생했는지 확인 |

연동 상태는 `GET /api/status`, 프로세스 상태는 `./scripts/iris-noa status`로 확인합니다. 로그는 Android 기기의 `/data/local/tmp/noa.log`에 기록됩니다.

## 문서

| 문서 | 내용 |
| --- | --- |
| [아키텍처](docs/1-architecture.md) | 구성 요소, 시작 순서, 인증, 전체 환경변수 |
| [데이터와 이벤트](docs/2-data-and-events.md) | DB 관찰, 이벤트 생성 및 감사 기록 |
| [메시지 전송](docs/3-delivery.md) | Intent, RichText, 파일, custom 전송 경로 |
| [에이전트](docs/4-agents.md) | 후킹·접근성 모드, LOCO, VOX 내부 구조 |
| [API 레퍼런스](docs/6-api.md) | Noa 및 Iris 요청·응답 명세 |
| [Iris 호환성](docs/7-iris-compatibility.md) | Iris 버전별 연동 점검 결과와 확인 범위 |

## 버그 보고 및 문의

버그와 기능 제안은 [GitHub Issues](https://github.com/entworacy/noa/issues)에서 받습니다. 버그 보고에는 Noa 버전 또는 커밋, 카카오톡·Android 버전, 기기 ABI, 재현 절차, 토큰과 개인정보를 제거한 관련 로그를 포함합니다.

기타 문의: [entworacy@gmail.com](mailto:entworacy@gmail.com)

## 라이선스

Copyright © 2026 Entworacy. [Noa License 1.0](LICENSE)이 적용됩니다.

개인·조직 내부 사용과 사적 수정은 허용됩니다. 소스·바이너리·수정본 재배포, 다른 제품에 Noa 바이너리를 포함한 배포, 소프트웨어 사본의 판매·대여·유상 설치에는 저작권자의 사전 서면 허가가 필요합니다. 자세한 허용 범위와 보증·책임 조건은 라이선스 전문을 따릅니다.

제3자 구성 요소의 고지와 라이선스는 [NOTICE](NOTICE) 및 [licenses/](licenses)에 정리되어 있습니다.
