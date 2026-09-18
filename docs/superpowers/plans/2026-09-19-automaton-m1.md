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
