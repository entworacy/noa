# 4. 후킹 및 접근성 에이전트

이전: [메시지·파일 전송 원리](3-delivery.md) · [README로 돌아가기](../README.md)

## 세 에이전트의 역할

Noa는 목적이 다른 세 어댑터를 사용합니다.

| 에이전트 | 실행 위치 | 역할 |
|---|---|---|
| Kakao agent | KakaoTalk 프로세스 | 내부 명령, LOCO·Room 관찰, VOX 제어와 송신 PCM 후킹 |
| Iris agent | Iris 프로세스 | `/reply` 가로채기와 `/noa` gateway |
| UI agent | 별도 UiAutomator 프로세스 | 화면 요소 탐색, 클릭, 상태 확인 |

Kakao/Iris 네이티브 에이전트는 최종 Noa 바이너리에 포함됩니다. Noa가 로컬 listener와 인증 token을 준비한 뒤 Frida를 이용해 대상 프로세스에 주입하고, 에이전트가 loopback TCP로 다시 연결합니다. 외부 네트워크에 에이전트 명령 포트를 공개하지 않습니다.

## Kakao 에이전트 초기화

Kakao agent는 현재 Android JavaVM을 찾고 스레드를 attach한 뒤 내장 DEX를 `InMemoryDexClassLoader`로 로드합니다. Java bridge의 native callback을 등록하고 LSPlant를 초기화한 다음 필요한 KakaoTalk 메서드만 정확한 이름·매개변수 목록으로 찾아 hook합니다.

초기화 상태는 JavaVM/loader/LSPlant 보유 상태와 모든 hook 설치가 끝난 준비 상태를 분리합니다. 중간 단계가 실패했는데 다음 연결에서 준비 완료로 오인하지 않습니다. 초기화 결과가 성공해야만 `ready` handshake를 전송합니다.

명령 protocol은 JSON line 기반이며 요청마다 token, ID, action을 포함합니다. 각 활성 ID는 독립된 operation, load 상태, 최종 결과와 condition variable을 갖습니다. 동일 ID가 진행 중이면 새 요청을 거부하고, callback 또는 timeout 뒤에는 RAII guard가 등록 상태를 제거합니다.

## 내부 작업

후킹 모드에서 Kakao agent는 다음 작업을 수행합니다.

- custom sending log를 로드하고 KakaoTalk sending manager 호출
- GETMEM으로 오픈채팅 멤버 저장소를 갱신한 뒤 강퇴 manager 호출
- 오픈프로필 link 조회 및 DB URL 대조
- 오픈채팅 URL 해석, 소유 프로필 적용, JOINLINK 호출과 결과 Room 검증
- 선택적 CHATONROOM 호출
- LOCO 송수신 packet capture
- Room database invalidation callback 전달
- 일반 보이스톡·오픈채팅 보이스룸 생성/입장/종료
- VOX WebRTC 마이크 frame에 명시적으로 공급된 PCM 교체 또는 혼합

KakaoTalk 내부 클래스와 메서드는 난독화될 수 있으므로 호출 전에 reflection으로 정확한 parameter type과 arity를 검사합니다. 결과 객체와 식별자도 null, 양수 범위, 요청값 일치 여부를 단계마다 확인합니다.

## LOCO 이벤트 경로

hook callback은 JNI 객체에서 header와 body를 추출해 구조화된 JSON 값으로 bounded queue에 넣습니다. 네트워크 송신 스레드는 token과 event type을 추가해 Noa의 event listener로 전송합니다. 캡처 스레드는 가득 찬 큐를 기다리지 않고 이벤트를 버릴 수 있어 KakaoTalk 실행 스레드를 막지 않습니다.

Noa는 최근 packet을 제한된 메모리 deque에 저장하고 `/api/loco`와 `/loco`에서 제공합니다. 이 기능은 진단용이며 완전한 패킷 보존이나 감사 로그를 보장하지 않습니다.

## VOX 제어와 오디오 송출

VOX 기능은 base의 `VoxModuleFacade` 계약, `vox_main` manager, `com.kakao.vox` SDK, WebRTC 순으로 내려가는 경계를 사용합니다. Java 어댑터는 Kakao 객체·callback과 UI 전환을 담당하고, Rust 에이전트는 명령 수명과 bounded PCM queue, JNI buffer 처리를 담당합니다. HTTP 검증과 KakaoTalk DB 조회는 호스트 Rust 서비스에만 둬 한 파일이나 한 언어에 역할이 몰리지 않게 분리합니다.

일반 보이스톡은 요청 직전 DB의 활성 참여자를 다시 확인해 caller와 peer 목록을 구성합니다. 오픈채팅 보이스룸 생성은 `OM` 방만 허용합니다. 입장에 필요한 `callId`, `csIP`, `csIP6`, `csPort`는 HTTP 요청에서 받지 않고 최신 type 52 `vr_invite` chat log에서 읽습니다. 최신 VOX log가 `vr_bye`이면 과거 초대 주소를 재사용하지 않습니다.

오디오 hook은 `AudioRecord.read(ByteBuffer, int)`의 원본 호출이 끝난 뒤 `AudioRecordJavaThread`에서만 활성화됩니다. 송출을 명시적으로 시작하지 않은 상태에서는 buffer를 전혀 바꾸지 않습니다. 형식은 48 kHz, mono, signed 16-bit little-endian PCM입니다.

- `replace`: 공급 PCM으로 마이크 frame을 교체합니다. queue가 비면 남은 부분을 0으로 채워 실제 마이크 음성이 새지 않습니다.
- `mix`: 실제 마이크 sample과 공급 PCM을 포화 덧셈합니다. queue가 비면 원래 마이크 sample을 유지합니다.

queue는 192,000바이트로 제한하며 넘치면 지연 누적 대신 가장 오래된 PCM을 버립니다. 한 push는 최대 96,000바이트이고 완전한 16-bit sample이어야 합니다. VOX hook 설치 실패는 채팅·LOCO hook 전체를 실패시키지 않지만 VOX audio 시작은 거부합니다.

## 접근성 경로

`KAKAO_HOOK_ENABLED=false`이면 공개 기능을 없애지 않고 지원 가능한 작업을 UI agent로 전환합니다. UI agent는 고정 좌표보다 resource-id, 텍스트, content description, 화면 구조를 사용합니다.

1. 알려진 화면은 화면을 여러 번 탐색하는 대신 정확한 Activity/Intent로 엽니다.
2. 상주 agent가 짧은 간격으로 의미 기반 selector를 검사합니다.
3. 클릭 직전에 대상 노드를 다시 찾아 새 메시지나 스크롤로 좌표가 변했는지 확인합니다.
4. 상주 agent 통신이 실패한 경우에만 XML dump 기반 복구 경로를 사용합니다.
5. 한국어·영어 label과 KakaoTalk 버전별 resource ID 묶음을 함께 관리합니다.

독립 실행형 `uiautomator runtest`와 호환되도록 레거시 `com.android.uiautomator` API를 사용합니다. 빌드 시 고정 Android 26 jar를 사용하고 시작 시 실제 기기 API와 필요한 JVM signature를 다시 검사합니다. 계약이 맞지 않으면 모호한 클릭을 시도하지 않습니다.

## 작업별 안전장치

### 프로필 공유

`linkId`가 있으면 DB의 활성 오픈프로필과 URL 형식을 먼저 검증합니다. 접근성 모드에서는 프로필 Activity를 열고 실제 공유/복사 control을 확인합니다. 멤버 `linkId`가 없으면 방과 대상 닉네임을 DB에서 해석하고 열린 프로필 이름을 재검증합니다. 같은 닉네임이 여러 명이면 `userId`를 화면에서 구분할 수 없으므로 중단합니다.

### 오픈채팅 입장

요청 URL과 소유 `profileId`를 서버에서 검증한 뒤 cover, 프로필 선택, 입장 완료, KakaoTalk 거부 문구를 서로 다른 상태로 인식합니다. 서버가 선택한 프로필과 같은 닉네임이 화면에 여러 개면 임의로 누르지 않습니다. 후킹 모드에서도 해석된 URL과 최종 Room의 link ID를 다시 대조합니다.

### 강퇴

닉네임만 전달했을 때 같은 이름이 여러 명이면 `userId` 사용을 요구합니다. 접근성 모드에서는 채팅 버블 좌표가 아니라 Room 멤버 Activity를 열고 대상 이름과 강퇴 control을 함께 확인합니다. 두 모드 모두 요청 전달 후 DB에서 대상 ID 제거를 확인해야 성공합니다.

### 나가기와 custom 재전송

나가기는 방 이름과 실제 나가기/확인 control을 확인합니다. custom 재전송은 방 제목, 실패 indicator와 같은 버블의 메시지·attachment, 재전송 확인 control을 순서대로 검증합니다. 화면 작업 전체는 하나의 직렬화 잠금 안에서 실행됩니다.

## 모드 차이

| 기능 | 후킹 활성화 | 후킹 비활성화 |
|---|---|---|
| 텍스트·파일·Markdown | Android Intent | Android Intent |
| custom | Kakao 내부 함수 + DB 검증 | UI 재전송 + DB 검증 |
| 오픈채팅 입장 | Kakao 내부 JOINLINK | 의미 기반 UI |
| 프로필 조회 | DB | DB |
| 프로필 공유 | 내부 조회 또는 UI + 검증 | UI + 검증 |
| 참여자 강퇴 | 내부 manager + DB 검증 | UI + DB 검증 |
| 채팅방 나가기 | 의미 기반 UI | 의미 기반 UI |
| 보이스톡·보이스룸 | VOX manager + DB 대상 검증 | 지원하지 않음 (`503`) |
| PCM 음성 송출 | VOX WebRTC capture 후킹 | 지원하지 않음 (`503`) |

후킹을 명시적으로 요청했는데 agent가 준비되지 않은 경우 접근성 경로로 조용히 전환하지 않습니다. 호출자가 실행 방식과 중복 실행 위험을 정확히 알 수 있도록 `503` 오류를 반환합니다.
