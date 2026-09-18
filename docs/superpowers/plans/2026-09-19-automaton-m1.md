# automaton M1 구현 계획 — Rust 코어 + Swift 셸 + 3모드 + Apprentice 1단계

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 스펙 `docs/superpowers/specs/2026-09-19-automaton-design.md` 의 M1 범위 — 오픈소스 모노리포 베이스(Rust 워크스페이스)에 코어 데몬·3모드·정책 엔진·메모리·Apprentice 1단계를 구현하고, SwiftUI 메뉴바 앱(Brass & Glass)으로 조작한다.

**Architecture:** 모든 로직은 Rust 크레이트(조립 가능 컴포넌트). 참조 데몬 `automatond`가 Unix domain socket에서 JSON-RPC를 제공하고 Swift 앱은 그 클라이언트다. 모든 툴 호출은 결정론적 Policy Engine을 경유한다.

**Tech Stack:** Rust 2024 edition (tokio, serde, jsonrpsee, rusqlite/FTS5, reqwest, objc2), Swift 6.4/SwiftUI (Xcode 27). 로컬 검증 툴체인: cargo 1.93.1.

**청크 구성:** Chunk 1 워크스페이스+프로토콜+정책 · Chunk 2 툴+에이전트 루프 · Chunk 3 메모리+스킬+Apprentice 1단계 · Chunk 4 참조 데몬+통합 테스트 · Chunk 5 Swift 앱(Brass & Glass)

---

## 파일 구조 (M1 전체 지도)

```
automaton/                       # 공개 모노리포 (Rust 워크스페이스)
├── Cargo.toml                   # workspace 정의
├── LICENSE-MIT · LICENSE-APACHE
├── crates/
│   ├── automaton-proto/         # JSON-RPC 타입 + 이벤트 (셸↔코어 공개 계약)
│   ├── automaton-policy/        # 결정론적 권한 엔진 + 정책 파일
│   ├── automaton-tools/         # Tool trait + 레지스트리 + coding/mac 툴
│   ├── automaton-memory/        # SQLite 세션·요약·사실 + 스킬 로더
│   ├── automaton-apprentice/    # 결정 저널 + 유사 결정 검색(1단계)
│   └── automaton-core/          # Provider trait + 에이전트 루프 + 모드/오케스트레이터
├── reference/automatond/        # 참조 데몬 (UDS JSON-RPC 서버 + doctor)
└── apps/Automaton/              # SwiftUI 메뉴바 앱 (Swift Package)
```

## Chunk 1: 워크스페이스 + automaton-proto + automaton-policy

### Task 1: 워크스페이스 스캐폴드

**Files:**
- Create: `Cargo.toml`, `.gitignore` (Rust 항목 추가), `LICENSE-MIT`, `LICENSE-APACHE`

- [ ] **Step 1: 워크스페이스 Cargo.toml 작성**

```toml
[workspace]
resolver = "2"
members = [
    "crates/automaton-proto",
    "crates/automaton-policy",
    "crates/automaton-tools",
    "crates/automaton-memory",
    "crates/automaton-apprentice",
    "crates/automaton-core",
    "reference/automatond",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT OR Apache-2.0"
repository = "https://github.com/TheMagicTower/automaton"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
toml = "0.9"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
async-trait = "0.1"
```

`.gitignore`에 추가:

```
/target
```

- [ ] **Step 2: 라이선스 전문 확보**

```bash
curl -fsSL https://www.apache.org/licenses/LICENSE-2.0.txt -o LICENSE-APACHE
curl -fsSL https://raw.githubusercontent.com/spdx/license-list-data/main/text/MIT.txt -o LICENSE-MIT
```
Expected: 두 파일 모두 1KB 이상.

- [ ] **Step 3: 멤버 크레이트 골격 생성 (이후 태스크에서 채움)**

```bash
for c in proto policy tools memory apprentice core; do
  cargo new crates/automaton-$c --lib --name automaton-$c
done
cargo new reference/automatond --name automatond
```
각 `crates/*/Cargo.toml`과 `reference/automatond/Cargo.toml`의 `cargo new`가 생성한 `[package]` 테이블을 아래 공통 상속 헤더로 **교체** (그대로 추가하면 TOML 중복 테이블 오류로 `cargo check` 실패):

```toml
[package]
name = "automaton-proto"  # 크레이트별 명시 이름 (name은 workspace 상속 불가) — 각 크레이트 이름으로 기입
version.workspace = true
edition.workspace = true
license.workspace = true
```

- [ ] **Step 4: 빌드 검증**

Run: `cargo check --workspace`
Expected: 멤버 전부 경고 없이 통과 (빈 골격이므로).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "chore: scaffold rust workspace with dual license"
```

### Task 2: automaton-proto — JSON-RPC 타입

**Files:**
- Create: `crates/automaton-proto/src/lib.rs`
- Test: `crates/automaton-proto/tests/roundtrip.rs`

- [ ] **Step 1: 직렬화 왕복 실패 테스트 작성**

`crates/automaton-proto/tests/roundtrip.rs`:

```rust
use automaton_proto::*;

#[test]
fn request_roundtrip() {
    let req = Request::MessageSend { session: "s1".into(), text: "다운로드 정리해줘".into() };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""message_send""#));
    assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);
}

#[test]
fn approval_request_event_roundtrip() {
    let ev = Event::ApprovalRequested {
        session: "s1".into(),
        approval: "a1".into(),
        action: ActionInfo { tool: "fs.delete".into(), target: "~/Downloads/old.zip".into(), risk: "파일 1개 삭제".into() },
        hint: Some(Hint { text: "지난번 유사 상황에서 승인(3회)".into(), similar_count: 3 }),
    };
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains(r#""approval_requested""#));
    assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), ev);
}

#[test]
fn mode_serializes_lowercase() {
    assert_eq!(serde_json::to_string(&Mode::Mac).unwrap(), r#""mac""#);
}

#[test]
fn unknown_event_type_is_error_not_panic() {
    assert!(serde_json::from_str::<Event>(r#"{"type":"future_thing"}"#).is_err());
}
```

- [ ] **Step 2: 테스트 실패 확인**

`crates/automaton-proto/Cargo.toml` 의존성:

```toml
[dependencies]  # cargo new가 생성한 빈 [dependencies] 테이블을 아래 내용으로 교체
serde.workspace = true
serde_json.workspace = true
```

Run: `cargo test -p automaton-proto`
Expected: FAIL — 타입 미정의 컴파일 오류.

- [ ] **Step 3: lib.rs 구현**

`crates/automaton-proto/src/lib.rs`:

```rust
//! automaton-proto — 셸↔코어 JSON-RPC 공개 계약 (텍스트 프로토콜, §4·§12)

use serde::{Deserialize, Serialize};

/// 세션 모드 (§5)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode { Code, Mac, Chat }

/// 셸 → 코어 요청
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    SessionCreate { id: String },
    MessageSend { session: String, text: String },
    ApprovalRespond { session: String, approval: String, decision: Decision, always: bool },
    ModeSwitch { session: String, to: Mode },
    SessionList,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision { Approve, Deny }

/// 승인 대상 동작 요약 (배너 1줄 표시용)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionInfo {
    pub tool: String,
    pub target: String,
    pub risk: String,
}

/// Apprentice 1단계 힌트 (§6)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hint {
    pub text: String,
    pub similar_count: u32,
}

/// 코어 → 셸 이벤트 (스트리밍)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    StreamDelta { session: String, delta: String },
    ToolStarted { session: String, tool: String, summary: String },
    ToolResult { session: String, tool: String, ok: bool, summary: String },
    ApprovalRequested { session: String, approval: String, action: ActionInfo, hint: Option<Hint> },
    ModeChanged { session: String, mode: Mode },
    Error { session: Option<String>, message: String },
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test -p automaton-proto`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(proto): json-rpc request/event contract types"
```

### Task 3: automaton-policy — 결정론적 권한 엔진

**Files:**
- Create: `crates/automaton-policy/src/lib.rs`
- Test: `crates/automaton-policy/tests/engine.rs`

- [ ] **Step 1: 실패 테스트 작성 (스펙 §5 위험 분류 테이블)**

`crates/automaton-policy/tests/engine.rs`:

```rust
use automaton_policy::*;

fn action(tool: &str, category: Category) -> Action {
    Action { tool: tool.into(), category, app: None, target: None }
}

#[test]
fn builtin_allows_reads_in_every_mode() {
    for mode in [Mode::Code, Mode::Mac, Mode::Chat] {
        let e = Engine::builtin();
        assert!(matches!(e.evaluate(&action("fs.read", Category::Read), mode), Verdict::Allow));
        assert!(matches!(e.evaluate(&action("capture.screen", Category::Read), mode), Verdict::Allow));
    }
}

#[test]
fn builtin_allows_code_mode_edits_but_asks_in_other_modes() {
    let e = Engine::builtin();
    assert!(matches!(e.evaluate(&action("fs.write", Category::Write), Mode::Code), Verdict::Allow));
    assert!(matches!(e.evaluate(&action("fs.write", Category::Write), Mode::Mac), Verdict::Ask { .. }));
}

#[test]
fn destructive_and_external_actions_always_ask() {
    for mode in [Mode::Code, Mode::Mac, Mode::Chat] {
        let e = Engine::builtin();
        assert!(matches!(e.evaluate(&action("fs.delete", Category::Destructive), mode), Verdict::Ask { .. }));
        assert!(matches!(e.evaluate(&action("shell.exec", Category::External), mode), Verdict::Ask { .. }));
    }
}

#[test]
fn input_defaults_to_ask_even_in_mac_mode_until_granted() {
    // 스펙 §5: 미등록 앱 클릭·타이핑 → ASK. 등록(=grant_always) 이후에만 Allow.
    let mut e = Engine::builtin();
    let a = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.apple.finder".into()), target: None };
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Ask { .. }));
    e.grant_always(Rule { name: "finder-input".into(), tool: Some("input.type".into()), app: Some("com.apple.finder".into()), category: None, verdict: VerdictTemplate::Allow });
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Allow));
}

#[test]
fn sensitive_targets_are_denied_regardless_of_rules() {
    let e = Engine::builtin();
    let a = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.some.bank".into()), target: Some("SecureTextField".into()) };
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Deny { .. }));
}

#[test]
fn deny_takes_precedence_over_allow_regardless_of_order() {
    let e = Engine::with_rules(vec![
        Rule { name: "allow-all-input".into(), tool: Some("input.type".into()), app: None, category: None, verdict: VerdictTemplate::Allow },
        Rule { name: "deny-example".into(), tool: None, app: Some("com.example.app".into()), category: None, verdict: VerdictTemplate::Deny },
    ]);
    let a = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.example.app".into()), target: None };
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Deny { .. }));
}

#[test]
fn mode_switch_always_asks_even_when_granted() {
    let e = Engine::builtin();
    assert!(matches!(e.evaluate(&action("mode.switch", Category::ModeSwitch), Mode::Chat), Verdict::Ask { .. }));
    // §5 원천 차단 불변식: 소유자 granted Allow 규칙으로도 모드 전환은 우회 불가
    let mut g = Engine::builtin();
    g.grant_always(Rule { name: "always-switch".into(), tool: Some("mode.switch".into()), app: None, category: None, verdict: VerdictTemplate::Allow });
    assert!(matches!(g.evaluate(&action("mode.switch", Category::ModeSwitch), Mode::Chat), Verdict::Ask { .. }));
}

#[test]
fn owner_grant_overrides_builtin_deny_app_but_never_password_fields() {
    // 스펙 §5: 민감 영역은 소유자가 명시적으로 등록(화이트리스트)한 경우에만 허용.
    let mut e = Engine::builtin();
    let bank = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.some.bank".into()), target: None };
    assert!(matches!(e.evaluate(&bank, Mode::Mac), Verdict::Deny { .. })); // builtin deny-app 규칙
    e.grant_always(Rule { name: "owner-trusts-bank".into(), tool: Some("input.type".into()), app: Some("com.some.bank".into()), category: None, verdict: VerdictTemplate::Allow });
    assert!(matches!(e.evaluate(&bank, Mode::Mac), Verdict::Allow)); // 명시 등록 → 허용
    let pw = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.some.bank".into()), target: Some("SecureTextField".into()) };
    assert!(matches!(e.evaluate(&pw, Mode::Mac), Verdict::Deny { .. })); // 비밀번호 필드는 절대 불가
}

#[test]
fn grant_always_persists_and_allows_future_matching() {
    let mut e = Engine::builtin();
    let a = action("fs.delete", Category::Destructive);
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Ask { .. }));
    e.grant_always(Rule { name: "trash-downloads".into(), tool: Some("fs.delete".into()), app: None, category: Some(Category::Destructive), verdict: VerdictTemplate::Allow });
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Allow));
}

#[test]
fn policy_file_roundtrip_persists_granted_rules() {
    let mut e = Engine::builtin();
    e.grant_always(Rule { name: "trash-downloads".into(), tool: Some("fs.delete".into()), app: None, category: Some(Category::Destructive), verdict: VerdictTemplate::Allow });
    let dir = std::env::temp_dir().join(format!("automaton-policy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("policy.toml");
    e.save(&path).unwrap();
    let loaded = Engine::from_file(&path).unwrap();
    assert!(matches!(loaded.evaluate(&action("fs.delete", Category::Destructive), Mode::Mac), Verdict::Allow));
}
```

- [ ] **Step 2: 테스트 실패 확인**

`crates/automaton-policy/Cargo.toml`:

```toml
[dependencies]  # 마찬가지로 기존 빈 테이블 교체
automaton-proto = { path = "../automaton-proto" }
serde.workspace = true
toml.workspace = true
thiserror.workspace = true
```

Run: `cargo test -p automaton-policy`
Expected: FAIL — 타입 미정의.

- [ ] **Step 3: lib.rs 구현**

`crates/automaton-policy/src/lib.rs`:

```rust
//! automaton-policy — 결정론적 권한 엔진 (§5). 학습 출력은 이 모듈을 우회할 수 없다.

pub use automaton_proto::Mode; // 재수출 — 하부 크레이트·테스트가 automaton_policy::Mode로 접근
use serde::{Deserialize, Serialize};

/// 툴 호출의 위험 분류 — 툴 구현이 자기 분류를 선언하고 엔진이 규칙으로 판정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category { Read, Write, Destructive, Input, External, ModeSwitch, System }

/// 정책 평가 대상 동작
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub tool: String,
    pub category: Category,
    pub app: Option<String>,
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerdictTemplate { Allow, Ask, Deny }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    pub tool: Option<String>,
    pub app: Option<String>,
    pub category: Option<Category>,
    pub verdict: VerdictTemplate,
}

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("io: {0}")] Io(#[from] std::io::Error),
    #[error("toml: {0}")] Toml(#[from] toml::de::Error),
    #[error("toml ser: {0}")] TomlSer(#[from] toml::ser::Error),
}

#[derive(Serialize, Deserialize)]
struct PolicyFile {
    #[serde(default)]
    granted: Vec<Rule>,
    #[serde(default)]
    rules: Vec<Rule>,
}

/// granted = 소유자 명시 서명("항상 허용") — builtin/파일 규칙보다 우선하되
/// 비밀번호 필드 정적 거부는 절대 우회 불가. rules = builtin + 정책 파일 규칙.
#[derive(Debug, Clone)]
pub struct Engine { granted: Vec<Rule>, rules: Vec<Rule> }

impl Engine {
    /// 스펙 §5 기본 위험 분류
    pub fn builtin() -> Self {
        let ask = |name: &str, category: Category| Rule {
            name: name.into(), tool: None, app: None, category: Some(category), verdict: VerdictTemplate::Ask,
        };
        let deny_app = |app: &str| Rule {
            name: format!("deny-app-{app}"), tool: None, app: Some(app.into()), category: None, verdict: VerdictTemplate::Deny,
        };
        Engine {
            granted: vec![],
            rules: vec![
                // DENY: 금지 앱 초기값 (소유자 grant_always로만 해제 가능, §5 민감 영역 화이트리스트)
                deny_app("com.some.bank"),
                deny_app("com.apple.MobileSMS"),
                // ASK: 파괴·외부·시스템·입력 (모드전환은 evaluate에서 무조건 Ask)
                ask("ask-destructive", Category::Destructive),
                ask("ask-external", Category::External),
                ask("ask-system", Category::System),
                ask("ask-input", Category::Input),
            ],
        }
    }

    pub fn with_rules(rules: Vec<Rule>) -> Self { Engine { granted: vec![], rules } }

    /// 결정론적 평가 순서:
    /// 1) 비밀번호 필드 → 정적 DENY (그 무엇도 우회 불가)
    /// 2) 모드 전환 → 항상 ASK (granted로도 우회 불가 — §5 원천 차단)
    /// 3) 소유자 명시 granted → Allow (§5 민감 영역 명시 등록 = 화이트리스트)
    /// 4) 명시 DENY 규칙 (규칙 목록 내 Allow보다 항상 우선)
    /// 5) 나머지 규칙 첫 매치
    /// 6) 카테고리×모드 기본값
    pub fn evaluate(&self, a: &Action, mode: Mode) -> Verdict {
        // 1. 민감 입력 필드 — 하드 거부
        if let Some(t) = &a.target {
            if t.contains("SecureTextField") || t.contains("Password") {
                return Verdict::Deny { reason: format!("민감 입력 필드: {t}") };
            }
        }
        // 2. 모드 전환은 언제나 승인 — 소유자 granted로도 우회 불가 (§5 원천 차단)
        if a.category == Category::ModeSwitch {
            return Verdict::Ask { reason: "모드 전환".into() };
        }
        // 3. 소유자 명시 허용 (파일 저장·재시작 후에도 유지)
        if self.granted.iter().any(|r| r.verdict == VerdictTemplate::Allow && matches(r, a)) {
            return Verdict::Allow;
        }
        // 4. 명시 DENY 규칙
        if let Some(r) = self.rules.iter().find(|r| r.verdict == VerdictTemplate::Deny && matches(r, a)) {
            return Verdict::Deny { reason: format!("규칙 {}: 거부", r.name) };
        }
        // 5. 나머지 규칙 — 첫 매치
        if let Some(r) = self.rules.iter().find(|r| r.verdict != VerdictTemplate::Deny && matches(r, a)) {
            return from_template(&r.verdict, a);
        }
        // 6. 카테고리×모드 기본값 — Input은 등록(granted) 전까지 모든 모드에서 ASK (§5 미등록 앱)
        match (a.category, mode) {
            (Category::Read, _) => Verdict::Allow,
            (Category::Write, Mode::Code) => Verdict::Allow,
            (Category::Write, _) => Verdict::Ask { reason: format!("{} 쓰기", short(a)) },
            (Category::Destructive, _) => Verdict::Ask { reason: format!("{} 파괴 동작", short(a)) },
            (Category::External, _) => Verdict::Ask { reason: format!("{} 외부 실행", short(a)) },
            (Category::System, _) => Verdict::Ask { reason: format!("{} 시스템 변경", short(a)) },
            (Category::Input, _) => Verdict::Ask { reason: format!("{} 미등록 앱 입력", short(a)) },
            (Category::ModeSwitch, _) => unreachable!(),
        }
    }

    /// "항상 허용" — granted에 추가, 정책 파일 저장 대상
    pub fn grant_always(&mut self, rule: Rule) { self.granted.push(rule); }

    pub fn from_file(path: &std::path::Path) -> Result<Self, PolicyError> {
        let raw = std::fs::read_to_string(path)?;
        let f: PolicyFile = toml::from_str(&raw)?;
        Ok(Engine { granted: f.granted, rules: f.rules })
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), PolicyError> {
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
        let f = PolicyFile { granted: self.granted.clone(), rules: self.rules.clone() };
        std::fs::write(path, toml::to_string(&f)?)?;
        Ok(())
    }
}

fn matches(r: &Rule, a: &Action) -> bool {
    let tool_ok = r.tool.as_ref().map_or(true, |t| t == &a.tool);
    let app_ok = r.app.as_ref().map_or(true, |g| a.app.as_ref() == Some(g));
    let cat_ok = r.category.map_or(true, |c| c == a.category);
    tool_ok && app_ok && cat_ok
}

fn from_template(t: &VerdictTemplate, a: &Action) -> Verdict {
    match t {
        VerdictTemplate::Allow => Verdict::Allow,
        VerdictTemplate::Ask => Verdict::Ask { reason: format!("{} 규칙 승인 필요", short(a)) },
        VerdictTemplate::Deny => Verdict::Deny { reason: format!("{} 규칙 거부", short(a)) },
    }
}

fn short(a: &Action) -> &str { &a.tool }
```

주의: ① 매처가 전부 None인 규칙은 캐치올이므로 builtin에 두지 않는다. ② DENY 경로는 비밀번호 필드(정적, 불가침)와 명시 deny 규칙뿐이며, 금지 앱은 deny 규칙으로 구현돼 소유자 grant_always로만 해제된다(§5 화이트리스트). ③ 모드 전환은 granted 평가 이전에 무조건 ASK — "항상 허용"으로도 우회 불가(§5 원천 차단). ④ 감사 로그(모든 정책 결정 기록)와 mac 모드 셸 허용 명령 allowlist는 Chunk 4(데몬)·Chunk 2(셸 툴)에서 구현한다.

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test -p automaton-policy`
Expected: 10 passed.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(policy): deterministic policy engine with builtin risk classification"
```

## Chunk 2: automaton-tools + automaton-core (에이전트 루프)

> 리뷰 자문 반영: 셸 명령 분류 가이드(스펙 §5 'code 모드 자율' 정합)를 Task 5에서, builtin+파일 정책 합성 시점은 Chunk 4에서 명시한다.

### Task 4: automaton-tools — Tool trait·레지스트리·fs/grep 툴

**Files:**
- Create: `crates/automaton-tools/src/lib.rs`, `crates/automaton-tools/src/fs_tools.rs`
- Test: `crates/automaton-tools/tests/registry_fs.rs`

- [ ] **Step 1: 실패 테스트 작성**

`crates/automaton-tools/tests/registry_fs.rs`:

```rust
use automaton_tools::*;
use serde_json::json;

fn tmp(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("automaton-tools-{}/", std::process::id())).join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

#[test]
fn registry_registers_and_look_up() {
    let mut r = Registry::new();
    r.register(Box::new(FsRead));
    assert_eq!(r.get("fs.read").unwrap().description(), "파일을 읽어 반환");
    assert!(r.get("fs.write").is_none());
    assert_eq!(r.names(), vec!["fs.read"]);
}

#[test]
fn fs_write_then_read_roundtrip() {
    let p = tmp("round.txt");
    FsWrite.execute(&json!({"path": p, "content": "홍브라스 시대"})).unwrap();
    let out = FsRead.execute(&json!({"path": p})).unwrap();
    assert!(out.contains("홍브라스"));
}

#[test]
fn fs_grep_reports_line_numbers() {
    let p = tmp("grep.txt");
    std::fs::write(&p, "alpha\nbeta brass\ngamma\n").unwrap();
    let out = FsGrep.execute(&json!({"path": p, "pattern": "brass"})).unwrap();
    assert!(out.contains("2:"), "실제 출력: {out}");
}

#[test]
fn fs_delete_removes_file_and_declares_destructive() {
    let p = tmp("del.txt");
    std::fs::write(&p, "x").unwrap();
    assert_eq!(FsDelete.category(&json!({"path": p})), automaton_policy::Category::Destructive);
    FsDelete.execute(&json!({"path": p})).unwrap();
    assert!(!p.exists());
}

#[test]
fn fs_read_declares_read_category_and_missing_file_is_error() {
    assert_eq!(FsRead.category(&json!({})), automaton_policy::Category::Read);
    assert!(FsRead.execute(&json!({"path": tmp("없는파일")})).is_err());
}
```

- [ ] **Step 2: 테스트 실패 확인**

`crates/automaton-tools/Cargo.toml` 의존성 (기존 빈 `[dependencies]` 교체):

```toml
automaton-policy = { path = "../automaton-policy" }
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
```

Run: `cargo test -p automaton-tools`
Expected: FAIL — 타입 미정의.

- [ ] **Step 3: lib.rs 구현 (trait·registry·edit)**

`crates/automaton-tools/src/lib.rs`:

```rust
//! automaton-tools — 툴 trait·레지스트리·coding 툴 (§4 Tool Registry)

pub mod fs_tools;
pub use fs_tools::*;

use automaton_policy::Category;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("{0}")] Message(String),
    #[error("io: {0}")] Io(#[from] std::io::Error),
}

/// 모든 툴의 계약. category(args)는 툴이 자기 위험 분류를 args 기반으로 선언 (§5).
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> serde_json::Value;
    fn category(&self, args: &serde_json::Value) -> Category;
    fn execute(&self, args: &serde_json::Value) -> Result<String, ToolError>;
}

pub struct Registry { tools: Vec<Box<dyn Tool>> }

impl Registry {
    pub fn new() -> Self { Registry { tools: vec![] } }
    pub fn register(&mut self, tool: Box<dyn Tool>) { self.tools.push(tool); }
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.iter().map(|t| t.as_ref() as &dyn Tool).find(|t| t.name() == name)
    }
    pub fn names(&self) -> Vec<&'static str> { self.tools.iter().map(|t| t.name()).collect() }
    /// 모드 툴셋 (§5) — chunk 4에서 mac 툴 추가
    pub fn coding_set() -> Self {
        let mut r = Registry::new();
        r.register(Box::new(FsRead));
        r.register(Box::new(FsWrite));
        r.register(Box::new(FsGrep));
        r.register(Box::new(FsDelete));
        r.register(Box::new(ShellExec)); // Task 5 완료 후 등록 (ShellExec는 Task 5에서 정의)
        r.register(Box::new(EditApply));
        r
    }
}

/// args에서 정책 Action의 app/target 필드를 추출 (없으면 None)
pub fn action_context(args: &serde_json::Value) -> (Option<String>, Option<String>) {
    (args.get("app").and_then(|v| v.as_str()).map(String::from),
     args.get("target").and_then(|v| v.as_str()).or_else(|| args.get("path").and_then(|v| v.as_str())).map(String::from))
}
```

`crates/automaton-tools/src/fs_tools.rs`:

```rust
//! coding 툴 — fs·grep·edit·shell (§5 code 모드 툴셋)

use crate::{Tool, ToolError};
use automaton_policy::Category;
use serde_json::Value;

fn arg_str(args: &Value, key: &str) -> Result<String, ToolError> {
    args.get(key).and_then(|v| v.as_str()).map(String::from).ok_or_else(|| ToolError::Message(format!("{key} 인자 누락")))
}

pub struct FsRead;
impl Tool for FsRead {
    fn name(&self) -> &'static str { "fs.read" }
    fn description(&self) -> &'static str { "파일을 읽어 반환" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})
    }
    fn category(&self, _args: &Value) -> Category { Category::Read }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        let s = std::fs::read_to_string(&path).map_err(|e| ToolError::Message(format!("읽기 실패 {path}: {e}")))?;
        const MAX: usize = 8 * 1024;
        if s.len() > MAX { Ok(format!("{}\n…(전체 {}바이트 중 앞부분)", &s[..MAX], s.len())) } else { Ok(s) }
    }
}

pub struct FsWrite;
impl Tool for FsWrite {
    fn name(&self) -> &'static str { "fs.write" }
    fn description(&self) -> &'static str { "파일에 내용을 쓴다" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]})
    }
    fn category(&self, _args: &Value) -> Category { Category::Write }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        let content = arg_str(args, "content")?;
        if let Some(dir) = std::path::Path::new(&path).parent() { std::fs::create_dir_all(dir)?; }
        std::fs::write(&path, &content).map_err(|e| ToolError::Message(format!("쓰기 실패 {path}: {e}")))?;
        Ok(format!("{path} 기록 완료 ({}바이트)", content.len()))
    }
}

pub struct FsGrep;
impl Tool for FsGrep {
    fn name(&self) -> &'static str { "fs.grep" }
    fn description(&self) -> &'static str { "파일에서 부분문자열을 찾아 '줄번호:내용' 목록 반환" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"pattern":{"type":"string"}},"required":["path","pattern"]})
    }
    fn category(&self, _args: &Value) -> Category { Category::Read }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        let pat = arg_str(args, "pattern")?;
        let s = std::fs::read_to_string(&path).map_err(|e| ToolError::Message(format!("읽기 실패 {path}: {e}")))?;
        let hits: Vec<String> = s.lines().enumerate().filter(|(_, l)| l.contains(&pat)).map(|(i, l)| format!("{}:{}", i + 1, l)).collect();
        Ok(if hits.is_empty() { format!("일치 없음: {pat}") } else { hits.join("\n") })
    }
}

pub struct FsDelete;
impl Tool for FsDelete {
    fn name(&self) -> &'static str { "fs.delete" }
    fn description(&self) -> &'static str { "파일을 삭제한다" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})
    }
    fn category(&self, _args: &Value) -> Category { Category::Destructive }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        std::fs::remove_file(&path).map_err(|e| ToolError::Message(format!("삭제 실패 {path}: {e}")))?;
        Ok(format!("{path} 삭제 완료"))
    }
}

pub struct EditApply;
impl Tool for EditApply {
    fn name(&self) -> &'static str { "edit.apply" }
    fn description(&self) -> &'static str { "파일 내 부분문자열을 치환한다 (모든 출현)" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"find":{"type":"string"},"replace":{"type":"string"}},"required":["path","find","replace"]})
    }
    fn category(&self, _args: &Value) -> Category { Category::Write }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        let find = arg_str(args, "find")?;
        let replace = arg_str(args, "replace")?;
        let s = std::fs::read_to_string(&path).map_err(|e| ToolError::Message(format!("읽기 실패 {path}: {e}")))?;
        if !s.contains(&find) { return Err(ToolError::Message(format!("찾을 문자열 없음: {find}"))); }
        let n = s.matches(&find).count();
        std::fs::write(&path, s.replace(&find, &replace))?;
        Ok(format!("{n}곳 치환 완료"))
    }
}
```

주의: `FsRead`의 `&s[..MAX]`는 UTF-8 경계에서 panic할 수 있다 — M1에선 다음 안전 절단 패턴으로 구현할 것:

```rust
let cut = MAX.min(s.len());
let cut = (0..=cut).rev().find(|i| s.is_char_boundary(*i)).unwrap();
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test -p automaton-tools`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(tools): tool trait, registry, coding fs/grep/edit tools"
```

### Task 5: shell 툴 — 명령 분류 가이드 (스펙 위임 사항)

**Files:**
- Create: `crates/automaton-tools/src/shell.rs`
- Modify: `crates/automaton-tools/src/lib.rs` (mod shell 선언)
- Test: `crates/automaton-tools/tests/shell.rs`

- [ ] **Step 1: 실패 테스트 작성**

`crates/automaton-tools/tests/shell.rs`:

```rust
use automaton_policy::Category;
use automaton_tools::{ShellExec, Tool};
use serde_json::json;

#[test]
fn read_only_commands_classify_read() {
    for cmd in ["ls -la", "cat notes.txt", "pwd", "git status", "git diff", "which cargo"] {
        assert_eq!(ShellExec.category(&json!({"command": cmd})), Category::Read, "{cmd}");
    }
}

#[test]
fn build_test_commands_classify_write() {
    for cmd in ["cargo build", "cargo test", "cargo check", "npm test", "make", "swift build"] {
        assert_eq!(ShellExec.category(&json!({"command": cmd})), Category::Write, "{cmd}");
    }
}

#[test]
fn everything_else_classifies_external() {
    for cmd in ["rm -rf /", "curl example.com", "osascript -e 'quit app'"] {
        assert_eq!(ShellExec.category(&json!({"command": cmd})), Category::External, "{cmd}");
    }
}

#[test]
fn executes_echo_and_reports_output() {
    let out = ShellExec.execute(&json!({"command": "echo brass"})).unwrap();
    assert!(out.contains("brass"), "실제 출력: {out}");
}

#[test]
fn missing_command_arg_is_error() {
    assert!(ShellExec.execute(&json!({})).is_err());
}
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test -p automaton-tools --test shell`
Expected: FAIL — ShellExec 미정의 (Task 4에서 선언만 존재).

- [ ] **Step 3: shell.rs 구현**

`crates/automaton-tools/src/shell.rs` (lib.rs에 `pub mod shell; pub use shell::*;` 추가):

```rust
//! shell.exec — 스펙 §5가 구현 계획에 위임한 '허용 명령 분류' 정의.
//! 분류 가이드(결정론적):
//!   READ   — 읽기 전용: ls, cat, head, tail, pwd, which, file, wc, git status/diff/log/show
//!   WRITE  — 빌드·테스트(부작용은 프로젝트 디렉토리 한정): cargo build/test/check, npm test, pnpm test, make, pytest, swift build/test
//!   EXTERNAL — 그 외 전부(네트워크·시스템 변경 가능): 항상 ASK. granted 규칙으로만 해제.
//! code 모드 자율성 = READ/WRITE가 Allow(§5), 위험 셸 = EXTERNAL이 ASK.

use crate::{Tool, ToolError};
use automaton_policy::Category;
use serde_json::Value;

const READ_PREFIXES: &[&str] = &["ls", "cat", "head", "tail", "pwd", "which", "file", "wc", "git status", "git diff", "git log", "git show"];
const WRITE_PREFIXES: &[&str] = &["cargo build", "cargo test", "cargo check", "npm test", "pnpm test", "make", "pytest", "swift build", "swift test"];

fn first_word_classify(cmd: &str) -> Category {
    let c = cmd.trim_start();
    if READ_PREFIXES.iter().any(|p| c == *p || c.starts_with(&format!("{p} "))) { Category::Read }
    else if WRITE_PREFIXES.iter().any(|p| c == *p || c.starts_with(&format!("{p} "))) { Category::Write }
    else { Category::External }
}

pub struct ShellExec;

impl Tool for ShellExec {
    fn name(&self) -> &'static str { "shell.exec" }
    fn description(&self) -> &'static str { "셸 명령을 실행하고 출력을 반환" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]})
    }
    fn category(&self, args: &Value) -> Category {
        args.get("command").and_then(|v| v.as_str()).map(first_word_classify).unwrap_or(Category::External)
    }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let command = args.get("command").and_then(|v| v.as_str()).ok_or_else(|| ToolError::Message("command 인자 누락".into()))?;
        let out = std::process::Command::new("/bin/sh").arg("-c").arg(command).output()
            .map_err(|e| ToolError::Message(format!("실행 실패: {e}")))?;
        let text = String::from_utf8_lossy(&out.stdout);
        let err = String::from_utf8_lossy(&out.stderr);
        Ok(format!("exit={} stdout:\n{} stderr:\n{}", out.status, text, err))
    }
}
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test -p automaton-tools`
Expected: 10 passed (기존 5 + shell 5).

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(tools): shell.exec with deterministic command classification guide"
```

### Task 6: automaton-core — Provider·모드 프로파일·에이전트 루프

**Files:**
- Create: `crates/automaton-core/src/lib.rs`, `crates/automaton-core/src/provider.rs`, `crates/automaton-core/src/loop_.rs`, `crates/automaton-core/src/mode.rs`
- Test: `crates/automaton-core/tests/loop_.rs`

- [ ] **Step 1: 실패 테스트 작성**

`crates/automaton-core/tests/loop_.rs`:

```rust
use automaton_core::*;
use automaton_policy::Engine;
use automaton_proto::{ActionInfo, Event, Mode};
use automaton_tools::Registry;
use serde_json::json;
use std::sync::{Arc, Mutex};

struct Scripted { turns: Mutex<Vec<Vec<StreamItem>>>, call: Mutex<usize> }
#[async_trait::async_trait]
impl Provider for Scripted {
    async fn complete(&self, _req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError> {
        let mut i = self.call.lock().unwrap();
        let t = self.turns.lock().unwrap();
        let items = t.get(*i).cloned().unwrap_or_default();
        *i += 1; // 턴 인덱스 전진 — 누락 시 모든 complete()가 turn 0 반환 (실측 결함 방지)
        Ok(items)
}

struct AutoGate(ApprovalOutcome);
#[async_trait::async_trait]
impl ApprovalGate for AutoGate {
    async fn decide(&self, _a: ActionInfo) -> ApprovalOutcome { self.0.clone() }
}

fn tmp(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("automaton-core-{}/", std::process::id())).join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

async fn run(provider: Box<dyn Provider>, gate: Box<dyn ApprovalGate>, user: &str) -> Vec<Event> {
    let mut events = vec![];
    let lp = AgentLoop::new(provider, gate, Engine::builtin(), Registry::coding_set(), Mode::Code);
    let mut history = vec![];
    let mut emit = |e: Event| events.push(e);
    lp.run_turn(&mut history, user.to_string(), &mut emit).await.unwrap();
    events
}

#[tokio::test]
async fn text_only_turn_streams_deltas() {
    let p = Scripted { turns: Mutex::new(vec![vec![StreamItem::Delta("안녕".into()), StreamItem::Delta("하세요".into())]]), call: Mutex::new(0) };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "인사해줘").await;
    assert!(ev.iter().filter(|e| matches!(e, Event::StreamDelta { .. })).count() >= 2);
    assert!(!ev.iter().any(|e| matches!(e, Event::ToolStarted { .. })));
}

#[tokio::test]
async fn allowed_tool_runs_and_result_recorded() {
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "fs.write".into(), args: json!({"path": tmp("a.txt"), "content": "brass"}) })],
            vec![StreamItem::Delta("기록했어".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "기록해").await;
    assert!(ev.iter().any(|e| matches!(e, Event::ToolStarted { tool, .. } if tool == "fs.write")));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: true, .. })));
    assert!(tmp("a.txt").exists());
}

#[tokio::test]
async fn destructive_tool_asks_then_executes_on_approval() {
    let f = tmp("del.txt");
    std::fs::write(&f, "x").unwrap();
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "fs.delete".into(), args: json!({"path": f}) })],
            vec![StreamItem::Delta("삭제 완료".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "삭제해").await;
    assert!(ev.iter().any(|e| matches!(e, Event::ApprovalRequested { .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: true, .. })));
    assert!(!f.exists());
}

#[tokio::test]
async fn denied_approval_leaves_file_and_reports_failure() {
    let f = tmp("keep.txt");
    std::fs::write(&f, "x").unwrap();
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "fs.delete".into(), args: json!({"path": f}) })],
            vec![StreamItem::Delta("취소됨".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Deny)), "삭제해").await;
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: false, .. })));
    assert!(f.exists());
}

#[tokio::test]
async fn policy_deny_skips_gate_entirely() {
    // 등록된 툴(fs.read)이지만 타깃 경로에 Password가 포함돼 정책 1단계 하드 거부 경로를 탄다.
    // (레지스트리에 없는 툴을 쓰면 unknown-tool 경로와 구분되지 않아 이 테스트가 무의미해짐)
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "fs.read".into(), args: json!({"path": tmp("Password.kdbx")}) })],
            vec![StreamItem::Delta("못 읽었어".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "비밀번호 파일 읽어줘").await;
    assert!(!ev.iter().any(|e| matches!(e, Event::ApprovalRequested { .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: false, .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { summary, .. } if summary.contains("민감 입력 필드"))));
}

#[tokio::test]
async fn unknown_tool_yields_error_result_and_loop_continues() {
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "없는툴".into(), args: json!({}) })],
            vec![StreamItem::Delta("계속".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "뭔가해줘").await;
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: false, .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::StreamDelta { delta, .. } if delta == "계속")));
}
```

- [ ] **Step 2: 테스트 실패 확인**

`crates/automaton-core/Cargo.toml` 의존성 (기존 빈 테이블 교체):

```toml
automaton-proto = { path = "../automaton-proto" }
automaton-policy = { path = "../automaton-policy" }
automaton-tools = { path = "../automaton-tools" }
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
async-trait.workspace = true
reqwest.workspace = true
```

Run: `cargo test -p automaton-core`
Expected: FAIL — 타입 미정의.

- [ ] **Step 3: 구현**

`crates/automaton-core/src/lib.rs`:

```rust
//! automaton-core — 에이전트 루프·모드 프로파일·프로바이더 (§4)

pub mod loop_;
pub mod mode;
pub mod provider;

pub use loop_::*;
pub use mode::*;
pub use provider::*;
```

`crates/automaton-core/src/provider.rs`:

```rust
//! Provider 계약 + 스크립티드/실제 구현. M1은 테스트용 스크립티드가 기본.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Message { pub role: String, pub content: String }

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall { pub name: String, pub args: Value }

#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem { Delta(String), ToolCall(ToolCall) }

pub struct CompletionRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<(String, String, Value)>, // (name, description, schema)
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{0}")] Message(String),
    #[error("provider: {0}")] Provider(String),
}

/// 프로바이더 계약 — 완성 스트림을 반환 (M1 단순화: Vec; 청크 스트리밍은 Chunk 4 데몬에서 개량)
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError>;
}

/// OpenAI 호환 HTTP 프로바이더 (키 재사용 — §3 Providers). M1 검증은 수동(env 키 필요), CI는 Scripted 사용.
pub struct OpenAiCompat { pub base_url: String, pub api_key: String, pub model: String, pub http: reqwest::Client }

impl OpenAiCompat {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("AUTOMATON_API_KEY").ok()?;
        let base_url = std::env::var("AUTOMATON_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into());
        let model = std::env::var("AUTOMATON_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
        Some(Self { base_url, api_key, model, http: reqwest::Client::new() })
    }
}

#[derive(serde::Deserialize)]
struct ChatResp { choices: Vec<Choice> }
#[derive(serde::Deserialize)]
struct Choice { message: RespMessage }
#[derive(serde::Deserialize)]
struct RespMessage { content: Option<String>, tool_calls: Option<Vec<RawToolCall>> }
#[derive(serde::Deserialize)]
struct RawToolCall { function: RawFunction }
#[derive(serde::Deserialize)]
struct RawFunction { name: String, arguments: String }

#[async_trait::async_trait]
impl Provider for OpenAiCompat {
    async fn complete(&self, req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError> {
        let tools: Vec<Value> = req.tools.iter().map(|(n, d, s)| serde_json::json!({
            "type": "function",
            "function": {"name": n, "description": d, "parameters": s}
        })).collect();
        let messages: Vec<Value> = std::iter::once(serde_json::json!({"role": "system", "content": req.system}))
            .chain(req.messages.iter().map(|m| serde_json::json!({"role": m.role, "content": m.content})))
            .collect();
        let body = serde_json::json!({"model": self.model, "messages": messages, "tools": tools});
        let resp = self.http.post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key).json(&body).send().await
            .map_err(|e| CoreError::Provider(e.to_string()))?;
        let chat: ChatResp = resp.json().await.map_err(|e| CoreError::Provider(e.to_string()))?;
        let m = chat.choices.into_iter().next().ok_or_else(|| CoreError::Provider("빈 응답".into()))?.message;
        let mut items = vec![];
        if let Some(t) = m.content { items.push(StreamItem::Delta(t)); }
        for c in m.tool_calls.unwrap_or_default() {
            let args: Value = serde_json::from_str(&c.function.arguments).unwrap_or(Value::Null);
            items.push(StreamItem::ToolCall(ToolCall { name: c.function.name, args }));
        }
        Ok(items)
    }
}
```

`crates/automaton-core/src/mode.rs`:

```rust
//! 모드 프로파일 (§5) — 툴셋+프롬프트 조합. 개인 조립은 이 정의를 데이터/코드로 재정의 (§2).

use automaton_proto::Mode;

pub struct ModeProfile { pub mode: Mode, pub tools: Vec<&'static str>, pub system_prompt: String }

impl ModeProfile {
    pub fn builtin(mode: Mode) -> Self {
        match mode {
            Mode::Code => ModeProfile {
                mode, tools: vec!["fs.read", "fs.write", "fs.grep", "fs.delete", "shell.exec", "edit.apply"],
                system_prompt: "당신은 automaton의 code 모드입니다. 정확한 엔지니어로서 파일을 읽고·쓰고·빌드해 작업을 완수하세요. 모든 도구 결과를 근거로 보고하세요.".into(),
            },
            Mode::Mac => ModeProfile {
                mode, tools: vec!["capture.screen", "ax.read", "input.click", "input.type", "shell.exec"],
                system_prompt: "당신은 automaton의 mac 모드입니다. 신중한 조작수로서 화면을 관찰하고 필요한 최소 동작만 수행하세요. 위험 동작은 승인 절차를 따릅니다.".into(),
            },
            Mode::Chat => ModeProfile {
                mode, tools: vec!["fs.read", "fs.grep"],
                system_prompt: "당신은 automaton의 chat 모드입니다. 대화 파트너로서 읽기 도구만으로 답변을 구성하세요.".into(),
            },
        }
    }
}
```

`crates/automaton-core/src/loop_.rs`:

```rust
//! 에이전트 루프 (§4·§9) — 스트리밍→툴콜→Policy→실행→결과 환류. 감사 로그 훅은 Chunk 4.

use crate::provider::{CompletionRequest, CoreError, Message, Provider, StreamItem, ToolCall};
use crate::mode::ModeProfile;
use automaton_policy::{Action, Engine, Verdict};
use automaton_proto::{ActionInfo, Event, Mode};
use automaton_tools::{action_context, Registry, Tool};
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalOutcome { Approve, Deny }

/// ASK 판정 시 승인을 구하는 계약 — 데몬은 JSON-RPC 승인 배너로, 테스트는 자동응답으로 구현.
#[async_trait::async_trait]
pub trait ApprovalGate: Send + Sync {
    async fn decide(&self, action: ActionInfo) -> ApprovalOutcome;
}

// Box<dyn ...>로 감싼 트레이트 객체가 그대로 트레이트를 만족하도록 전달 구현 (테스트·데몬에서 Box 사용)
#[async_trait::async_trait]
impl<P: Provider + ?Sized> Provider for Box<P> {
    async fn complete(&self, req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError> { (**self).complete(req).await }
}

#[async_trait::async_trait]
impl<G: ApprovalGate + ?Sized> ApprovalGate for Box<G> {
    async fn decide(&self, action: ActionInfo) -> ApprovalOutcome { (**self).decide(action).await }
}

pub struct AgentLoop<P: Provider, G: ApprovalGate> {
    provider: P,
    gate: G,
    policy: Engine,
    registry: Registry,
    profile: ModeProfile,
    approval_seq: AtomicU32,
}

impl<P: Provider, G: ApprovalGate> AgentLoop<P, G> {
    pub fn new(provider: P, gate: G, policy: Engine, registry: Registry, mode: Mode) -> Self {
        let profile = ModeProfile::builtin(mode);
        AgentLoop { provider, gate, policy, registry, profile, approval_seq: AtomicU32::new(1) }
    }

    pub async fn run_turn(&self, history: &mut Vec<Message>, user: String, emit: &mut dyn FnMut(Event)) -> Result<(), CoreError> {
        const MAX_TURNS: usize = 32; // 비정상 프로바이더 무한 반복 방지 (§9)
        let session = "s".to_string(); // 세션 ID는 Chunk 4 데몬이 주입
        history.push(Message { role: "user".into(), content: user });
        let mut consecutive_failures: usize = 0; // §9: 동일 툴 연속 실패 중단의 루프 수준 근사
        for _ in 0..MAX_TURNS {
            let req = self.build_request(history);
            let items = self.provider.complete(req).await?;
            let mut assistant = String::new();
            let mut calls = vec![];
            for it in items {
                match it {
                    StreamItem::Delta(d) => { emit(Event::StreamDelta { session: session.clone(), delta: d.clone() }); assistant.push_str(&d); }
                    StreamItem::ToolCall(c) => calls.push(c),
                }
            }
            history.push(Message { role: "assistant".into(), content: assistant });
            if calls.is_empty() { return Ok(()); }
            for call in calls {
                let before = history.len();
                self.run_tool_call(&session, &call, history, emit).await?;
                let failed = matches!(history[before..].last(), Some(m) if m.content.contains("오류:") || m.content.contains("거부됨:") || m.content.contains("거절됨:"));
                consecutive_failures = if failed { consecutive_failures + 1 } else { 0 };
                if consecutive_failures >= 3 {
                    return Err(CoreError::Message("동일 툴 연속 3회 실패 — 중단 (§9)".into()));
                }
            }
        }
        Err(CoreError::Message(format!("최대 턴 {MAX_TURNS} 초과 — 중단")))
    }

    fn build_request(&self, history: &[Message]) -> CompletionRequest {
        let tools = self.profile.tools.iter().filter_map(|n| self.registry.get(n)).map(|t| (t.name().to_string(), t.description().to_string(), t.parameters_schema())).collect();
        CompletionRequest { system: self.profile.system_prompt.clone(), messages: history.to_vec(), tools }
    }

    async fn run_tool_call(&self, session: &str, call: &ToolCall, history: &mut Vec<Message>, emit: &mut dyn FnMut(Event)) -> Result<(), CoreError> {
        let Some(tool) = self.registry.get(&call.name) else {
            let msg = format!("알 수 없는 툴: {}", call.name);
            emit(Event::ToolResult { session: session.into(), tool: call.name.clone(), ok: false, summary: msg.clone() });
            history.push(Message { role: "tool".into(), content: format!("[{}] 오류: {msg}", call.name) });
            return Ok(());
        };
        let (app, target) = action_context(&call.args);
        let action = Action { tool: call.name.clone(), category: tool.category(&call.args), app, target: target.clone() };
        let verdict = self.policy.evaluate(&action, self.profile.mode);
        let info = ActionInfo { tool: call.name.clone(), target: target.unwrap_or_default(), risk: summary_of(&action) };
        match verdict {
            Verdict::Allow => { self.execute_and_record(session, tool, call, history, emit); }
            Verdict::Ask { reason } => {
                let id = self.approval_seq.fetch_add(1, Ordering::SeqCst).to_string();
                emit(Event::ApprovalRequested { session: session.into(), approval: id, action: ActionInfo { risk: reason.clone(), ..info.clone() }, hint: None });
                let outcome = self.gate.decide(info).await;
                match outcome {
                    ApprovalOutcome::Approve => self.execute_and_record(session, tool, call, history, emit),
                    ApprovalOutcome::Deny => {
                        let msg = "사용자가 거절했습니다.".to_string();
                        emit(Event::ToolResult { session: session.into(), tool: call.name.clone(), ok: false, summary: msg.clone() });
                        history.push(Message { role: "tool".into(), content: format!("[{}] 거절됨: {msg}", call.name) });
                    }
                }
            }
            Verdict::Deny { reason } => {
                let msg = format!("정책 거부: {reason}");
                emit(Event::ToolResult { session: session.into(), tool: call.name.clone(), ok: false, summary: msg.clone() });
                history.push(Message { role: "tool".into(), content: format!("[{}] 거부됨: {msg}", call.name) });
            }
        }
        Ok(())
    }

    fn execute_and_record(&self, session: &str, tool: &dyn Tool, call: &ToolCall, history: &mut Vec<Message>, emit: &mut dyn FnMut(Event)) {
        emit(Event::ToolStarted { session: session.into(), tool: call.name.clone(), summary: tool.description().to_string() });
        let result = tool.execute(&call.args).map_err(|e| e.to_string());
        let (ok, text) = match result { Ok(s) => (true, s), Err(e) => (false, e) };
        emit(Event::ToolResult { session: session.into(), tool: call.name.clone(), ok, summary: truncate(&text, 400) });
        history.push(Message { role: "tool".into(), content: format!("[{}] {}", call.name, text) });
    }
}

fn summary_of(a: &Action) -> String { format!("{} {}", a.tool, a.target.clone().unwrap_or_default()).trim().into() }

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n { s.into() } else { format!("{}…", &s[..s.floor_char_boundary(n)]) }
}
```

주의: ① `floor_char_boundary`은 아직 nightly 전용일 수 있다 — 컴파일 오류 시 `(0..=n).rev().find(|i| s.is_char_boundary(*i)).unwrap()` 절단으로 교체할 것 (FsRead와 동일 패턴). ② `tool.execute`는 동기 블로킹 — M1 수용, Chunk 4에서 `spawn_blocking` 래핑. ③ 감사 로그(모든 정책 결정 기록)·builtin+파일 정책 합성은 Chunk 4 데몬의 책임. ④ 스펙 §5 code 툴셋의 `lsp`는 M1에서 제외, 후속 계획으로 지연. ⑤ proto의 `ApprovalRespond{always:true}`는 게이트가 아니라 Chunk 4 데몬이 `engine.grant_always`를 직접 호출하는 경로로 처리한다(게이트는 1회성 승인만 반환). ⑥ OpenAiCompat은 tool_call_id 없는 role:tool 직렬화라 실 엔드포인트에서 400 가능 — Chunk 4에서 실제 툴콜 직렬화(tool_call_id 포함)로 보강할 것. M1 검증은 Scripted 프로바이더로 수행한다.

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test -p automaton-core`
Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat(core): agent loop with policy gate, approval flow, mode profiles"
```
