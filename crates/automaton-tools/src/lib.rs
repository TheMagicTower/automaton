//! automaton-tools — 툴 trait·레지스트리·coding/mac 툴 (§4 Tool Registry, §5 툴셋)

pub mod fs_tools;
pub use fs_tools::*;
pub mod shell;
pub use shell::*;
pub mod mac_tools;
pub use mac_tools::*;

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
    /// mac 모드 툴셋 (§5)
    pub fn mac_set() -> Self {
        let mut r = Registry::new();
        r.register(Box::new(CaptureScreen));
        r.register(Box::new(AxRead));
        r.register(Box::new(InputClick));
        r.register(Box::new(InputType));
        r.register(Box::new(ShellExec));
        r
    }
    /// coding 모드 툴셋 (§5)
    pub fn coding_set() -> Self {
        let mut r = Registry::new();
        r.register(Box::new(FsRead));
        r.register(Box::new(FsWrite));
        r.register(Box::new(FsGrep));
        r.register(Box::new(FsDelete));
        r.register(Box::new(ShellExec));
        r.register(Box::new(EditApply));
        r
    }
}

/// args에서 정책 Action의 app/target 필드를 도구별 선언 피연산자 기반으로 추출.
/// 도구별 명시 피연산자만 추출하므로 미선언 decoy 키(decoy path, decoy target 등)나
/// 타입 혼동(비문자열 키)이 실제 실행 인자를 마스킹하거나 정책 엔진 검사를 가로채는 우회를 원천 방지한다.
pub fn action_context(tool_name: &str, args: &serde_json::Value) -> (Option<String>, Option<String>) {
    let app = args.get("app").and_then(|v| v.as_str()).map(String::from);
    let target = match tool_name {
        "shell.exec" => args.get("command").and_then(|v| v.as_str()),
        t if t.starts_with("fs.") || t.starts_with("edit.") => args.get("path").and_then(|v| v.as_str()),
        t if t.starts_with("input.") => args.get("target").and_then(|v| v.as_str()),
        _ => args.get("path").and_then(|v| v.as_str())
            .or_else(|| args.get("command").and_then(|v| v.as_str()))
            .or_else(|| args.get("target").and_then(|v| v.as_str())),
    }.map(String::from);
    (app, target)
}
