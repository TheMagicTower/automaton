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
        // 1. 민감 입력 필드 — 하드 거부 (대소문자 무관, 하드코딩된 민감 키워드 검사)
        if let Some(t) = &a.target {
            let lower = t.to_lowercase();
            if lower.contains("securetextfield") || lower.contains("password") || lower.contains("passcode") || lower.contains("secret") || lower.contains("pin") {
                return Verdict::Deny { reason: format!("민감 입력 필드: {t}") };
            }
        }
        // 2. 모드 전환은 언제나 승인 — 소유자 granted로도 우회 불가 (§5 원천 차단)
        if a.category == Category::ModeSwitch {
            return Verdict::Ask { reason: "모드 전환".into() };
        }
        // 3. 소유자 명시 앱 특정 허용:
        // 앱 특정 명시 허용(r.app.is_some())은 명시 DENY도 오버라이드 (§5 소유자 화이트리스트).
        if self.granted.iter().any(|r| r.verdict == VerdictTemplate::Allow && r.app.is_some() && matches(r, a)) {
            return Verdict::Allow;
        }
        // 4. 명시 DENY 규칙 (앱 미지정 일반 허용보다 항상 우선하여 은행/금지 앱 보호)
        if let Some(r) = self.rules.iter().find(|r| r.verdict == VerdictTemplate::Deny && matches(r, a)) {
            return Verdict::Deny { reason: format!("규칙 {}: 거부", r.name) };
        }
        // 4.5. 앱 미지정 일반 소유자 허용 (비-DENY 앱/카테고리만 허용)
        if self.granted.iter().any(|r| r.verdict == VerdictTemplate::Allow && matches(r, a)) {
            return Verdict::Allow;
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
