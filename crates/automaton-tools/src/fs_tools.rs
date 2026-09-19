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
        if s.len() > MAX {
            let cut = (0..=MAX).rev().find(|i| s.is_char_boundary(*i)).unwrap(); // UTF-8 안전 절단
            Ok(format!("{}\n…(전체 {}바이트 중 앞부분)", &s[..cut], s.len()))
        } else { Ok(s) }
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
