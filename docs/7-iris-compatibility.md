# 7. Iris 호환성 점검

[README로 돌아가기](../README.md)

## 대상 버전

2026년 11월 2일 확인 기준 최신 정식 릴리스는 [Iris v0.33](https://github.com/dolidolih/Iris/releases/tag/v0.33)입니다. 공개일은 2026년 10월 25일이며, 기본 브랜치 `main`도 릴리스와 같은 커밋 `194c7eb91b5556bbc411427c1dccf10ae49f0f7f`를 가리켰습니다.

Noa 설치 스크립트는 Iris 버전을 고정하지 않습니다. Iris가 없으면 공식 `iris_control`로 최신 릴리스를 설치하고, `/data/local/tmp/Iris.apk`가 이미 있으면 기존 파일을 유지합니다. 따라서 Noa 소스만으로 기기에 설치된 Iris 버전을 알 수 없습니다. 이번 점검에는 테스트 기기를 사용하지 않았습니다.

공식 릴리스의 APK를 내려받아 GitHub가 제공한 SHA-256과 일치하는지 확인했습니다.

| 릴리스 | APK SHA-256 |
| --- | --- |
| v0.33 | `c4f61523ac41353f97c3547e12f32bf3a9819d838c489b2150224a8f1f47932f` |
| v0.32 | `00eaa7800f904b7e3082277525adbbc0581ffe9efdb1c7a475561361f918eab0` |

## 발견한 차이와 수정

수정 전 Noa의 `/reply` 역직렬화 코드는 `ReplyType` enum을 받는 생성자에 의존했습니다. Iris v0.33의 실제 APK에는 첫 번째 인자가 `String`인 생성자가 있어, 기존 호출은 생성자를 찾지 못합니다. 텍스트·이미지를 포함한 `/reply` 요청에 영향을 줍니다.

| 항목 | Iris v0.33 | Noa 대응 |
| --- | --- | --- |
| `ReplyRequest.type` | enum에서 문자열로 변경 | 문자열 생성자를 우선 사용하고 구버전 enum 생성자도 지원 |
| 전송 타입 | `media`, `video`, `audio`, `file` 추가 | Noa 확장 요청 외의 타입은 Iris 처리기로 전달 |
| `file.data` | 객체·배열 및 Base64 문자열 지원 | 객체·배열은 Iris로 전달. Noa의 Base64·Data URI·경로 전송 유지 |
| `Replier.sendMessage` | 인자와 반환형 유지 | 기존 후킹 지점 유지 |
| Ktor | 3.1.1 사용 | 라우팅·본문 읽기·응답 함수의 DEX 시그니처 일치 확인 |
| 실행 방식 | `party.qwer.iris.Main`, 기존 APK 경로 유지 | 설치·프로세스 탐색 방식과 일치 |
| `/reply` 응답 | 비동기 접수 결과 `Enqueued` | 전송 완료 또는 DB 확인 응답으로 해석하지 않도록 API 문서 수정 |

Iris의 신규 조회·설정 경로는 Noa의 기본 `/noa` 접두사와 겹치지 않습니다. Noa는 접두사 밖의 경로를 원래 Iris 라우팅으로 넘깁니다. 사용자 지정 접두사는 Iris 경로와 겹치지 않게 선택해야 합니다.

## 검증 결과

- v0.33과 v0.32 공식 APK에서 생성자, 후킹 대상, Kotlin serialization, Ktor 연동에 사용하는 메서드와 정적 필드를 확인했습니다.
- 실제 Noa Java 요청 어댑터를 enum 모델과 문자열 모델 각각에 연결한 22개 검증을 통과했습니다. 이 테스트의 Iris 모델은 APK에서 확인한 시그니처를 재현한 fixture입니다.
- Rust 요청 분기 테스트 3개를 통과했습니다. 새 미디어 형식 전달, 기존 Noa 확장 전송 선택, 후킹 비활성화와 요청 필드 검증을 포함합니다.
- Noa 본체의 기존 Rust 테스트 59개를 통과했습니다.
- 수정한 Iris 에이전트의 DEX 생성과 Android ARM64 컴파일을 통과했습니다.

검증 코드는 `scripts/check-iris-apk.py`, `iris-agent/tests/reply_compat.py`, `iris-agent/src/reply.rs`에 있습니다.

## 확인 범위

소스와 배포 APK 기준으로 확인한 `/reply` 호환성 문제는 수정했습니다. 실제 Android에서의 주입, ART 후킹, HTTP 요청부터 카카오톡 수신까지의 전송은 검증하지 않았습니다.

v0.33 APK의 SQLCipher 라이브러리는 `arm64-v8a`와 `x86_64`만 포함합니다. 32비트 환경에서 다른 패키지의 라이브러리를 사용하는 대체 경로까지 정상 동작한다고 판단할 근거는 없습니다. SQLCipher 로딩, 암호화 DB 접근, Android 권한과 키 저장소 접근은 기기에서 확인해야 합니다.

Iris의 신규 DB 조회 구현은 Noa 자체 DB 읽기 코드와 별개입니다. Iris 업데이트만으로 Noa의 DB 조회나 custom 전송 검증이 보장되지는 않습니다.

## 근거

- [공식 릴리스](https://github.com/dolidolih/Iris/releases/tag/v0.33)
- [ReplyRequest 모델](https://github.com/dolidolih/Iris/blob/v0.33/app/src/main/java/party/qwer/iris/model/ReplyRequest.kt)
- [Iris HTTP 서버와 전송 처리](https://github.com/dolidolih/Iris/blob/v0.33/app/src/main/java/party/qwer/iris/IrisServer.kt)
- [공식 설치 스크립트](https://github.com/dolidolih/Iris/blob/v0.33/iris_control)
- [SQLCipher 라이브러리 로딩](https://github.com/dolidolih/Iris/blob/v0.33/app/src/main/java/party/qwer/iris/SqlCipherNative.kt)
