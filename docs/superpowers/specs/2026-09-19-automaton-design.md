# automaton — 설계 문서

날짜: 2026-09-19 · 상태: 승인됨 (브레인스토밍 5섹션 확정 + 이원화 전략 개정)

## 1. 개요

**automaton**은 macOS(Apple Silicon) 전용 오픈소스 GUI 에이전트 **컴포넌트 라이브러리 집합**이다. 코딩과 맥 조작을 모두 수행하는 모드 기반 개인 에이전트 하네스의 기반 구성요소를 제공하고, 온디바이스 학습 모듈(Apprentice Engine)이 사용자의 의사결정을 학습해 개인화한다. 소유자의 개인 하네스는 이 크레이트들을 불러와 조립한 별도 코어로 구축한다(§2).

### 목표
- 코딩(code) · 맥 조작(mac) · 대화(chat) 모드를 지닌 완전한 개인 에이전트
- 사용자 의사결정 학습 → 승인 힌트 → 답장 자동완성(클릭 수락)으로 발전
- 미려한 네이티브 UI (Brass & Glass — 다크 월넛 + 황동 + 세리프, 기본 테마)
- 모든 상태·자격증명은 로컬 (온디바이스)
- **조립 가능한 베이스**: 제3자가 크레이트를 조합해 자신만의 하네스를 코드 수준에서 구축
- **음성 대화**: 글로벌 핫키/웨이크워드로 말하고 듣는 조작(자비스급 대화를 목표, §12 단계 로드맵)

### 비목표
- macOS 외 플랫폼, 멀티유저, 원격 호스팅
- 메신저 게이트웨이·cron 스케줄러 (필요시 후속 프로젝트)
- 학습 모듈의 자율 실행 권한 (참고용 어드바이저로 한정)
- 개인 프롬프트·개인 데이터의 베이스 포함 (개인 하네스 전용)

## 2. 이원화 구조 (오픈소스 컴포넌트 + 개인 조립)

| | 베이스 (공개) | 개인 하네스 (비공개) |
|---|---|---|
| 저장소 | `TheMagicTower/automaton` (MIT OR Apache-2.0) | `TheMagicTower/automaton-personal` (private) |
| 내용 | 구성요소 크레이트(루프·정책·툴·메모리·Apprentice·프로토콜) + 참조 구현(데몬 `automatond`·SwiftUI 앱) + 프로토콜 명세 | 베이스 크레이트를 의존성으로 **내 코어 바이너리를 조립** — 개인 모드 정의·정책 프리셋·커스텀 툴·성격 프롬프트를 코드로 결합 |
| 형태 | Rust 워크스페이스(크레이트별 게시) + Swift 앱 | Rust 바이너리 + `~/.config/automaton/` 설정·스킬 데이터 |
| 데이터 | 없음 (콘텐츠 중립) | 개인 메모리 DB·정책 파일·결정 저널 (git 미관리) |

- 조립이 코드 수준에서 가능하려면 모든 개인화 포인트(모드 정의, 정책 규칙, 툴, 프롬프트, 테마)가 베이스에서 **trait·빌더·설정 파일 인터페이스**로 노출되어야 한다. 이것이 이원화의 실현 조건이다.
- 참조 구현(`automatond`·기본 SwiftUI 앱)은 베이스 사용법을 보여주는 동작 예제이자 그 자체로 쓸 수 있는 기본 하네스다.
- **단일 모노리포**: 베이스는 하나의 저장소(Rust 워크스페이스)로 제공 — clone 한 번으로 참조 데몬·Swift 앱을 빌드·실행할 수 있고 `cargo install --git` 설치를 지원해 퍼가기 진입장벽을 최소화한다. 생태계 경로: 제3자가 베이스 크레이트 위에 자신의 코어·스킬·테마·모드 팩을 만들고 공유(agentskills.io 호환 + 테마/모드 팩 표준). 베이스 개선이 모든 파생 하네스로 회수됨.
- 개인 메모리 DB(`~/.local/share/automaton/`)는 어떤 저장소에도 커밋되지 않는다.

## 3. 핵심 결정

| 결정 | 내용 |
|---|---|
| 접근 방식 | 전용 Rust 하네스 (기존 플랫폼 의존 없음). 모델 키만 기존 계정 재사용 |
| 프로세스 구조 | 코어는 독립 데몬(참조 구현 `automatond`), SwiftUI 앱은 JSON-RPC 클라이언트 |
| 모드 | 툴셋+프롬프트+권한 프로파일 조합(데이터·코드로 정의). 세션 리셋 없이 툴셋만 스왑 |
| 권한 | 결정론적 Policy Engine이 최종 판단. 학습 출력은 우회 불가 |
| 학습 | 이벤트 기반 Apprentice Engine, 루프와 분리(비동기) |
| UI | SwiftUI 메뉴바 앱(팝오버 주창구) + 보조 메인 윈도우 |
| 라이선스 | MIT OR Apache-2.0 (Rust 생태계 관례) |

## 4. 아키텍처

```
SwiftUI Shell (메뉴바 앱)
  대화 스트림 · 모드 인디케이터 · 승인 배너(과거 결정 힌트) · 답장 초안 칩
        ↕ JSON-RPC over Unix domain socket
Rust 코어 데몬 (참조 구현: automatond / 개인: automaton-personal이 조립한 바이너리)
  Session Orchestrator  모드별 프롬프트·툴셋·권한 프로파일
  Agent Loop            tool-calling 루프 · 스트리밍 · 컨텍스트 압축
  Policy Engine         결정론적 위험 판정 → ALLOW/ASK/DENY (최종 권한)
  Providers             OpenAI 호환 HTTP 클라이언트 · 기존 키 재사용
  Tool Registry         coding(fs·shell·edit·grep·lsp) / mac(capture·AX·click·type·제한 셸)
  Memory · Skills       SQLite + FTS5 + sqlite-vec, 마크다운 스킬
  Apprentice Engine     Decision Journal · 임베딩 서비스 · Draft Composer
        ↕ macOS API
Accessibility API(AXUIElement) · ScreenCaptureKit · CGEvent
```

- 코어가 데몬이므로 셸이 죽어도 세션이 살고, CLI/TUI를 나중에 얹을 수 있으며 headless 테스트가 가능하다.
- Swift↔Rust 경계는 JSON-RPC 한 겹으로 최소화한다.
- 코딩 툴과 맥 툴은 같은 레지스트리에 등록되고, 모드는 그 부분집합이다.
- 각 박스(Orchestrator·Loop·Policy·Registry·Memory·Apprentice)는 독립 크레이트로 분리되어 개별 의존·재조립이 가능하다.

## 5. 모드·권한 시스템

초기 3 모드 (베이스가 제공하는 기본값, 개인 조립에서 재정의 가능):

| 모드 | 툴셋 | 자율 | 성격 |
|---|---|---|---|
| code | fs·shell·edit·grep·lsp | 자율 (위험 셸만 검사) | 정확한 엔지니어 |
| mac | capture·AX·click·type + 제한 셸 | 위험 동작 승인 | 신중한 조작수 |
| chat | 읽기 전용(grep·read) | 완전 자율 | 대화 파트너 |

- 모드 전환: 사용자 명시(단축키·명령) 또는 에이전트 요청 → 전환 자체가 승인 이벤트. 에이전트가 자율적으로 자율성을 높이는 것을 원천 차단.
- 권한 흐름: 툴 호출 → Policy Engine(규칙 평가) → ALLOW(즉시 실행) / ASK(승인 배너, Apprentice 힌트 표시, 승인·거절·항상 허용) / DENY(거절 사유 반환).
- 위험 분류 기본값 — ALLOW: 캡처, AX 읽기, 파일 읽기·검색, code 모드 편집·빌드. ASK: mac 모드 셸(허용 명령 allowlist는 구현 계획에서 정의), 파일 삭제/이동, 시스템 설정, 미등록 앱 클릭·타이핑, 모드 전환, 외부 전송·결제 전 전부. DENY: 금지 목록 + 민감 영역(비밀번호 필드·뱅킹 앱 등)은 **소유자가 명시적으로 화이트리스트에 등록한 경우에만 허용, 기본 거부**.
- "항상 허용"은 규칙 단위로 정책 파일에 기록(감사 가능, 언제든 철회). 학습 힌트와 달리 이 파일이 권한의 원천이다.
- 모든 정책 결정(allow 포함)은 감사 로그 + Decision Journal에 기록되어 Apprentice 학습 데이터가 된다.

## 6. Apprentice Engine (온디바이스 학습)

이벤트 버스를 구독해 대화 내역·승인/거절·제안 수락을 학습한다. 루프와 분리되어 에이전트 성능에 영향을 주지 않는다. 출력은 언제나 참고용(어드바이저)이며 Policy Engine을 우회할 수 없다.

3단계 로드맵:

1. **결정 저널 + 유사 결정 검색** — kNN만 사용(학습 없음). 승인 배너에 "지난번 유사 상황에서 승인(3회)" 힌트. 즉시 가치.
2. **선호 스코어러** — 임베딩 위 소형 분류기로 P(승인) 예측 → 배너 기본값 사전 선택. 정책 우회 없음.
3. **답장 자동완성** — 로컬 소형 LM(1~2B, llama.cpp Metal)이 내 답변 초안 생성 → 입력창 칩 → 클릭 수락. 성공률 충분할 때 활성화 게이트.

기술: 소형 gguf/ONNX 임베더(수십 MB) → sqlite-vec. 생성 단계부터 llama.cpp(Metal) + 가중치 1~2GB 추가.

## 7. 메모리·스킬

역할 분리 — 스킬=명시적 절차 기억(어떻게), 메모리=사실(무엇을), Apprentice=암시적 선호(무엇을 좋아하는지).

메모리 3계층 (단일 SQLite: sessions·summaries·facts·decisions):
1. **세션** — 현재 대화·툴 결과, 압축 요약으로 관리
2. **작업** — 세션 요약 보관, FTS5 + 벡터 검색
3. **장기(큐레이션)** — 세션 종료/유휴 시 에이전트가 기억 제안 → 사용자 승인만 저장(오염 방지). 거절 이력도 Apprentice가 학습해 같은 제안 반복 방지

스킬 (agentskills.io 호환):
- 폴더 + `SKILL.md`(이름·설명·사용시점) + 선택적 스크립트/참고문서
- 점진적 로딩: 시스템 프롬프트엔 이름·설명만, 필요 시 본문 로드
- 위치: `~/.config/automaton/skills/`(전역) + 프로젝트 `.automaton/skills/`, 모드 스코프 가능
- 자기개선 루프: 복잡 작업 성공 후 절차를 스킬로 제안 → 승인 저장

## 8. UI — Brass & Glass

- SwiftUI 메뉴바 앱. 팝오버가 주 창구: 모드 스위처(⚙code·🔭mac·📖chat), 대화 스트림, 승인 배너(Apprentice 힌트 포함), 답장 초안 칩(클릭 수락).
- 보조 메인 윈도우: 세션 히스토리, 메모리 브라우저, 정책 파일 편집, 설정.
- 비주얼: 다크 월넛(#1a1611 계열) 배경 + 황동(#b08d57/#c9a227) 액센트 + 세리프 제목(Georgia 계열). 가독성 유지, 다크 모드 자연스러움.
- 테마는 파일로 정의되어 교체 가능(개인 하네스·제3자 팩 모두). 글로벌 단축키·오버레이 프리뷰(mac 모드에서 조작 대상 하이라이트).
- 음성 입출력은 §12. 셸 기능으로 구현(코어 프로토콜은 텍스트 유지).

## 9. 데이터 흐름·에러 처리

흐름: 입력 →(JSON-RPC)→ Session Orchestrator → Agent Loop(스트리밍→툴콜→Policy→실행→결과 환류) → 각 단계 이벤트가 버스로 UI·Decision Journal·감사 로그에 동시 전파 → 응답 후 Apprentice 비동기 후처리(임베딩 저장).

에러 처리:
- Provider 장애 — 지수 백오프 3회 재시도, 실패 시 세션 보존+알림. 세션은 SQLite 지속이라 데몬 재시작 후 속개.
- 툴 실패 — 오류를 모델에 반환해 자가 판단. 동일 툴 연속 3회 실패 시 중단+보고.
- 승인 무응답 — 자동 승인 없이 대기 유지.
- AX/TCC 권한 상실 — 시작 시 자가진단(`automaton doctor`)으로 치유 안내.
- 크래시 — WAL 저널로 마지막 일관 상태 복구.

## 10. 테스트 전략

- Policy Engine: 결정론적 규칙 → 테이블 완전 커버 + proptest 불변식(학습 출력이 권한 우회 불가, DENY 영역은 모드 무관 거부 등).
- Agent Loop: mock provider로 녹화된 툴콜 시퀀스 재생 → headless CI 검증.
- AX·ScreenCaptureKit 실기기 테스트: 권한 필요, 로컬 전용 통합 테스트로 분리.
- 보안: 키는 Keychain(디스크 평문 없음), 감사 로그 무결성.
- SwiftUI 셸: 스모크 수준(주요 플로우 자동화·수동 체크리스트)이라도 계획에 포함.

## 11. 기술 스택·레이아웃 (초기)

- Rust: tokio, serde/serde_json, jsonrpsee(UDS), rusqlite + sqlite-vec, reqwest(OpenAI 호환), llama-cpp-2(3단계), objc2(AX·CGEvent·ScreenCaptureKit 바인딩)
- Swift: SwiftUI 메뉴바 앱 (Swift Package, 코어와는 UDS로 통신)
- **베이스 저장소(공개)**: `crates/automaton-core`(에이전트 루프·오케스트레이터) · `crates/automaton-policy` · `crates/automaton-tools`(레지스트리+coding/mac 툴) · `crates/automaton-memory` · `crates/automaton-apprentice` · `crates/automaton-proto`(JSON-RPC 명세 — 제3자 클라이언트·코어 조립의 공개 계약) · `reference/automatond`(참조 데몬) · `apps/Automaton`(참조 SwiftUI 앱). 크레이트는 crates.io에 개별 게시.
- **개인 저장소(비공개)**: `automaton-personal` — 베이스 크레이트(git 의존성)로 내 코어를 조립하는 바이너리: 개인 모드·정책 프리셋·커스텀 툴·성격 프롬프트. 설정·스킬은 `~/.config/automaton/`에 배포.
- 라이선스: MIT OR Apache-2.0

## 12. 음성 대화 (자비스 로드맵)

핵심 구조 결정: **음성은 셸(클라이언트) 기능이다.** 코어 데몬·JSON-RPC 프로토콜은 텍스트를 유지하고, Swift 셸이 오디오 파이프라인(마이크 캡처·VAD·STT·TTS 재생)을 담당한다. 음성을 얹어도 코어·학습·권한 구조가 그대로 재사용된다.

검토 결론(2026-09 기준 부품 모두 성숙):
- **STT** — whisper.cpp(Rust 바인딩 whisper-rs) + Silero VAD, 온디바이스 한국어 우수. 대안: Apple Speech Framework(무료·온디바이스).
- **TTS** — Piper 1.7(ONNX, 한국어 보이스, Apple Silicon 실측 양호). 스트리밍 청크 재생으로 체감 지연 절감.
- **웨이크워드** — openWakeWord(ONNX), "automaton" 커스텀 워드 학습 가능.

단계 로드맵 (Apprentice와 동일한 게이트 방식):
1. **PTT 대화** — 글로벌 핫키 push-to-talk → STT → 기존 에이전트 루프 → TTS 스트리밍. 체감 지연 목표 1.5s 이내(스트리밍 병렬화로 달성). 승인 배너 음성 낭독 포함.
2. **상시청각 + 바지인** — 웨이크워드 상시 감지, 말하기 시작하면 TTS 즉시 중단(VAD 바지인), 승인을 음성으로("응"/"아니") 응답.
3. **자비스급 실험** — realtime speech-to-speech API 경로 추가(지연 500ms 목표). 이 단계에서도 셸이 오디오 세션을 맡고 함수 호출만 텍스트 JSON-RPC로 코어에 브리징해 "코어 프로토콜 텍스트 유지" 원칙은 그대로 성립. 온디바이스 풀듀플렉스는 이 단계에서 재평가.

프라이버시: STT·TTS 기본 온디바이스. 클라우드 음성 API는 명시적 선택 시에만(3단계).

계획 스코프: 첫 구현 계획은 코어+Swift 셸+3모드+Apprentice 1단계까지. 음성(PTT 포함)은 별도 후속 계획으로 분리하며, whisper.cpp를 Swift C API로 직접 호출할지 Rust 헬퍼를 둘지는 그 계획에서 확정한다.

## 13. 참고

- OpenClaw — 신뢰 게이트웨이·결정론적 정책 철학, 스킬/플러그인 구조
- Hermes Agent (Nous Research) — 큐레이션 메모리, 자기개선 스킬 루프 패턴
- axcli / gridhand / macmatic — macOS AX 자동화 사례
