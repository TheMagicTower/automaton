//! automaton-tools — 툴 trait·레지스트리·coding 툴 (§4 Tool Registry)

pub mod fs_tools;
pub use fs_tools::*;
pub mod shell;
pub use shell::*;

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
        r.register(Box::new(ShellExec));
        r.register(Box::new(EditApply));
        r
    }
}

/// args에서 정책 Action의 app/target 필드를 추출 (없으면 None)
pub fn action_context(args: &serde_json::Value) -> (Option<String>, Option<String>) {
    (args.get("app").and_then(|v| v.as_str()).map(String::from),
     args.get("target").and_then(|v| v.as_str()).or_else(|| args.get("path").and_then(|v| v.as_str())).map(String::from))
}
