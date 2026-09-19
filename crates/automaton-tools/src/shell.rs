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
    const METACHARS: &[&str] = &[";", "&&", "||", "|", ">", ">>", "<", "&", "`", "$(", "\n", "\r"];
    let c = cmd.trim_start();
    // 복합 명령 우회 방지: 메타문자 포함 시 무조건 External (항상 ASK)
    if METACHARS.iter().any(|m| c.contains(m)) { return Category::External; }
    // 파일 출력/쓰기 플래그 우회 방지 (예: git diff/log/show --output=...)
    if c.contains("--output") || c.contains(" -o ") || c.contains(" -o=") || c.ends_with(" -o") {
        return Category::External;
    }
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
