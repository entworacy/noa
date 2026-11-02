# KakaoTalk 26.9.1 호환성 점검

2026-10-28 최초 검사, 2026-10-29 추가 검사. Noa 소스 `1321333` 기준으로 APK를 비교하고 VOX 팩토리·LOCO 패킷 읽기·채팅방 관리자 접근을 수정했다. APK 정적 검사는 실제 ART/LSPlant 후킹 성공을 보장하지 않는다.

## 분석 대상

APKPure에서 버전 코드를 지정해 XAPK를 내려받고 base APK의 Android manifest와 DEX를 검사했다. APK와 역컴파일 결과는 저장소에 포함하지 않는다.

| 항목 | 기준 버전 | 새 버전 |
|---|---|---|
| 패키지 | `com.kakao.talk` | `com.kakao.talk` |
| versionName / versionCode | `26.6.3` / `29260630` | `26.9.1` / `29260910` |
| base APK SHA-256 | `58e6701ff1a5b4e91c76bd63e3493aad7ba69cdeef9f4e254de251b4dfaa2e3a` | `90a3a9b08b246478ec8d1ae71d94c1a2558f04a23828bf50212f90c1324c6549` |
| DEX 클래스 수 | 201,831 | 225,749 |

두 APK에서 추출한 v2 서명 인증서 SHA-256은 `2b06cc3d47782d7c497c07f17cb5f859cd6bbcb66829f3e67b96b7a44820d2ce`로 일치한다. 인증서 지문 비교이며 전체 APK 서명의 암호학적 검증을 대신하지 않는다. 26.9.1 다운로드 번들은 ARM64, minSdk 32 변형이다. 다른 배포 변형까지 검사한 것은 아니다.

다운로드: [26.9.1 XAPK](https://d.apkpure.net/b/XAPK/com.kakao.talk?versionCode=29260910), [26.6.3 XAPK](https://d.apkpure.net/b/XAPK/com.kakao.talk?versionCode=29260630).

## 발견 및 수정

VOX 팩토리 클래스가 다음과 같이 이동했다.

- 이전: `com.kakao.talk.vox.VoxModuleFacadeFactory`
- 신규: `com.kakao.talk.vox.facade.VoxFacadeModuleFactory`

26.9.1에는 이전 클래스가 없어 기존 `VoxController.facade()`가 `ClassNotFoundException`으로 실패한다. 새 경로를 우선 탐색하고 클래스가 없는 경우 이전 경로를 사용하는 방식으로 수정했다. 생성 메서드는 `android.content.Context` 매개변수와 `VoxModuleFacade` 반환형을 확인한다. 새 경로의 초기화 오류를 구버전 경로로 숨기지 않는다. 내장 `assets/noa-kakao-agent.dex`도 다시 생성했다.

VOX manager 인터페이스는 `contract`에서 `contract.internal` 패키지로 이동했지만, Noa는 facade 반환 객체를 통해 호출하므로 이 패키지명을 별도로 변경할 필요가 없다.

## 확인한 계약

- 실제 Noa Rust 인덱서: 두 APK 모두 최초 검사 필수 소스 시그니처 14종과 `chat-on-room`, `join-link`, `prepare-resend` 후보 탐색 통과. 메서드 후보 발견은 실제 명령 실행 성공과 다르다.
- LOCO 구조: 이전 `gt.h.y(mt.k)` → 신규 `tl70.x(dr70)` 송신, 이전 `gt.h$b.b(mt.l, Continuation)` → 신규 `tl70$b.b(er70, Continuation)` 수신. 공통 packet 부모 및 factory 형태가 유지된다. 기존 동적 탐색을 유지했다.
- VOX: 9인자 CecallMakingInfo 생성자, 통화 생성·종료, 보이스룸 생성·종료·입장 시그니처 유지. 입장 메서드 존재 검사는 실제 manager 객체 연결 확인을 대신하지 않는다.
- MasterDatabase·SecondaryDatabase 및 VOX Activity·OutgoingTrigger 클래스 존재 확인.

## 검증 결과

- 기존 코드 + 신규 팩토리만 있는 Java fixture: ClassNotFoundException 재현.
- 수정 코드 + 구/신 팩토리 fixture: 2개 회귀 테스트 통과. 실제 `VoxController.facade()`를 호출하고 캐시 재사용도 검사한다.
- Kakao 에이전트 Rust 테스트 13개 통과. 외부 APK를 요구하는 테스트 1개는 기본 실행에서 제외되며 두 실제 APK에 대해 별도로 통과했다.
- Java DEX 재빌드 및 ARM64 네이티브 에이전트 릴리즈 빌드 확인.

## 남은 실행 검증

이 환경에는 연결된 테스트 기기가 없다. 실제 26.9.1 프로세스 주입, ART/LSPlant 초기화, 동적 reflection 전체 선택, Room 무효화 알림, 메시지·오픈채팅 명령, VOX 통화 및 PCM 송출은 미검증이다. 실제 DB 스키마와 서버 응답도 APK의 타입 검사만으로 보장할 수 없다. 테스트 기기에서 에이전트 ready, LOCO 송수신, 각 명령 결과 및 VOX 상태를 확인해야 정상 동작을 확정할 수 있다.

이 수정은 앞서 생성한 v1.4.3 릴리즈 태그/바이너리에 포함되지 않는다.


## 2026-10-29 추가 점검과 수정

최초 검사는 LOCO 훅 **대상 탐색**을 확인했지만 콜백의 패킷 필드 읽기와 실제 채팅방 관리자 인스턴스 획득을 충분히 확인하지 못했다. 추가로 아래 두 호환성 문제를 확인하고 수정했다. 전체 후킹이 정상이라고 판정한 것은 아니다.

| 경로 | 26.6.3 | 26.9.1 | 영향 및 조치 |
|---|---|---|---|
| LOCO 헤더/본문 타입 | `mt.b` / `mt.a` | `on70` / `il70` | Rust 캡처가 이전 타입을 고정 참조해 실패. DEX 소스 시그니처에 연결된 패킷 필드와 검증된 레이아웃을 읽는 `LocoPacketReader`로 변경 |
| 채팅방 관리자 인스턴스 | `Yr.c1$a.j()` Companion getter | `iga.a()` (`ChatRoomListManagerProvider.kt`) → App component → `gga` | 기존 singleton 탐색 실패로 방 조회·전송·오픈채팅 작업에 영향. 기존 경로를 유지하고 타입과 인자를 검증한 Provider 지원 |

`iga.a()` 바이트코드는 실제 App component에서 manager를 가져와 초기화 메서드를 호출한 후 반환한다. 의존성을 직접 생성하거나 임의 객체를 찾지 않고 이 앱의 Provider를 호출한다. Provider가 여러 개이거나 null을 반환하면 실패 처리한다.

LOCO 헤더 생성자 및 `toString` 바이트코드에서 `a:int`=packetId, `b:short`=status, `c:enum`=method, `d:int`=bodyLength임을 두 버전에서 확인했다. 본문 `a` 필드는 BSON 객체다. 이 필드 이름·타입·생성자 계약은 계속 검증하며, 타입 이름을 무조건 추정하지 않는다. 새로운 읽기 계약을 LOCO 대상 탐색 단계에서도 검사하므로 설치 이후에야 타입 오류가 반복되는 문제를 줄인다. 알 수 없는 구조나 중복 header/body 필드는 거부한다.

DEX 인덱스에는 `LocoHeader.kt`, `LocoBody.kt`를 추가했다. `ChatRoomListManagerProvider.kt`는 구버전에 없으므로 선택적 소스다. 필수 소스 16종 및 세 작업의 인덱스 테스트가 두 APK 모두 통과했다.

### 추가로 비교한 범위

| 항목 | APK 비교 결과 | 판단 범위 |
|---|---|---|
| GETMEM | `Yr.l0$a.m1` → `xw9$a.a1`, `(long,List,Continuation) → Object` 유지 | 기존 구조 탐색 조건 유지 |
| 채팅방 생성 | manager 메서드 `W` → `O`, `(OpenLink,String,String,String)` 유지 | 인스턴스 획득 경로 수정 필요, 인자 계약 유지 |
| custom 재전송 | Request companion `u(room,ChatSendingLog,mode,listener,boolean)` 유지 | callback 성공 `(object,long,object)` 및 실패 형태 유지 |
| 강퇴·숨김 | foreground의 `(room,Friend,boolean,boolean,listener)`와 `(long,room,List,List,List)` 유지 | listener의 두 무인자 메서드도 유지; 실제 성공·실패 응답은 미검증 |
| 오픈프로필 | `(long,JSONObject)` 요청, profile factory 형태, OpenLink `getName/getUrl` 유지 | 난독화 이름 변경은 기존 탐색으로 대응 |
| suspend context | `EmptyCoroutineContext.C` 유지 | 현재 고정 필드 참조가 두 APK에 존재 |
| Room 감시 | tracker accessor와 LiveData 구현 이름 변경, `(String[],boolean,Callable)` factory 유지 | Noa는 형태로 탐색하므로 별도 이름 변경 불필요; 실제 observer 등록 미검증 |
| VOX 상태 | CallInfo·CurrentInfo·하위 객체의 필드 타입/순서 유지 | 기존 상태 추출 형태 유지; runtime reflection 순서는 실기기 확인 필요 |
| RichText | ChatSendingLog에 RichText 필드 추가 | Noa가 사용하는 `getId` 및 재전송 계약 유지. 새 스타일 기능 구현/전송을 검증한 것은 아님 |
| DB DDL 문자열 | chat_logs의 컬럼 순서 차이 및 과거 생성 SQL 정리 확인 | 추출된 chat_logs/chat_rooms/chat_sending_logs/open_link/open_chat_member/open_profile 생성 SQL의 컬럼 집합에 Noa용 필수 컬럼 삭제를 발견하지 못함. 실제 마이그레이션 DB는 미검증 |

추가 검증:

- `scripts/test-kakao-contracts.sh`: 구/신 packet의 요청·응답, 상속 필드, 중복/미지원 packet 거부, legacy singleton, 새 Provider, 모호한/null Provider 거부. 10개 확인 통과.
- Rust 13개 테스트 및 외부 APK 인덱스 테스트(각 버전) 통과.
- 확대된 `scripts/check-kakao-apk.py` 검사 두 버전 통과. 패킷 읽기 레이아웃과 manager 인스턴스 경로도 검사한다.
- VOX 회귀 테스트 2개, 내장 DEX 재생성 및 ARM64 에이전트 릴리즈 빌드 확인.

이 결과는 **검사한 정적 계약 범위**의 결과다. 모든 앱 변경이나 런타임 부작용이 없다는 의미는 아니며, 기기 없이 정상 후킹 전체를 확정할 수 없다. 세 수정 모두 작업 브랜치에만 있고 기존 릴리즈 바이너리에는 포함되지 않는다.


## 2026-11-02 Markdown → RichText 전송 변경

앞선 RichText 점검은 기존 재전송 계약의 유지 여부만 다뤘다. 이번에는 Iris `type: "markdown"` 입력을 실제 RichText attachment로 변환하도록 전송 경로를 변경했다. API 입력은 유지하고 `markdown: true`를 제거했다. 지원 문법과 예시는 [전송 문서](3-delivery.md)를 참고한다.

같은 26.9.1 APK에서 추가로 확인한 근거:

- `RichText$a.<clinit>`의 serializer 키는 `bold`, `italic`, `strike`이며 모두 기본값이 있는 목록이다.
- `RichTextRange`는 `loc`, `length`를 가지며 `wkb1.c`가 두 정수를 순서대로 JSON 배열로 직렬화한다.
- `com.kakao.talk.chat.richtext.a$a.b(Spanned)`는 `getSpanStart`, `getSpanEnd`와 그 차이로 범위를 만든다. 따라서 바이트나 Unicode code point가 아닌 UTF-16 단위다. `c(List)`는 같은 서식의 겹치거나 이어진 범위를 병합한다.
- `y0g(Intent)`는 `EXTRA_CHAT_MESSAGE`, `EXTRA_CHAT_ATTACHMENT`, 타입을 읽고 `a(listener,long)`에서 ChatSendingLog builder에 전달한다. `ShareManager.f0` 역시 텍스트 경로에서 attachment JSON을 전달한다.
- `ChatSendingLog.f2`는 `richText`를 `eq9.x`로 파싱한다. `ChatSendingLog.U`는 `richText`를 `getJSONObject`로 읽어 전송 attachment에 복사하므로 JSON 문자열이 아닌 중첩 객체를 넣는다.

검증: 변환 테스트 6개를 포함한 루트 Rust 테스트 59개 및 ARM64 Noa 릴리즈 빌드 통과. 4,096개 문법/Unicode 조합에서 패닉 없이 유효한 UTF-16 범위를 생성하는 것도 검사했다. 실제 Android 수신·서식 표시 확인은 미실시이며, 기존 릴리즈에 이 변경이 포함된 것은 아니다.


## LOCO 직접 발신 API 추가

`POST /api/loco/send` → control `send-loco` → JNI 타입 변환 → `LocoSender` → 관찰된 앱 client 송신 메서드 경로를 추가했다. 외부 소켓이나 별도 로그인 세션은 생성하지 않는다.

추가 정적 근거:

- 26.9.1 `dr70$a`(이전 `mt.k$a`) 요청 빌더는 `(LocoMethod,int,short)` 생성자, `(String,Object) → builder` 필드 추가, `() → request` 생성 메서드를 가진다. BSON 인코더의 출력 길이를 헤더에 넣는다.
- `LocoJob$b`의 유일한 무인자 int 메서드는 앱의 AtomicInteger를 증가시키는 공용 ID 발급기다. 별도의 카운터로 ID를 생성하지 않는다.
- 26.9.1 `tl70.u()`는 disposed 플래그와 socket 연결 상태를 확인한다. `tl70.x(request)`는 패킷 바이트를 socket에 쓰고 flush한다. 같은 client monitor에서 일반 송신과 직접 송신을 직렬화한다.
- 수신 루프는 응답 ID로 대기 객체를 찾아 기존 receive hook을 호출한다. 직접 송신은 이 대기 객체를 만들지 않으므로, 응답을 해당 hook에서 관찰할 수 있다고 주장하지 않는다.
- 확대된 Python 검사와 실제 Rust DEX 인덱스 테스트를 두 APK 모두 통과했다. `LocoReq.kt`, `LocoJob.kt`가 새 탐색 소스다.

회귀 검사: 루트 Rust 59개, Kakao 에이전트 일반 테스트 14개, 공용 프로토콜 테스트 4개 통과. 내장 DEX 및 ARM64 Kakao 에이전트·Noa 릴리즈 빌드도 통과했다. 별도 실제 JVM 테스트(`-Xcheck:jni`)에서 int64 최대값, int32/long/double 구분, null, 바이너리, 한글·이모지·NUL, 3,000항목 배열의 JNI 참조 수명을 확인했다. Java fixture는 실제 sender/Hooker/packet reader를 컴파일하여 패킷 생성, 공용 ID 발급, 연결 거부와 교체, 송신 오류 전달, 캡처, 동시 송신을 검증했다. 기존 packet/provider 10개 및 VOX 2개 회귀 검사도 통과했다.

실제 앱의 ART hook 경유 송신과 카카오톡 서버의 응답은 테스트 기기가 없어 미검증이다. `verification: "socket-write"`, `serverConfirmed: false`로 이 범위를 표현한다. 기존 릴리즈에는 아직 포함되지 않았다.
