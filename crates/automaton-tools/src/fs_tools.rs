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

pub struct FsReadLines;
impl Tool for FsReadLines {
    fn name(&self) -> &'static str { "fs.read_lines" }
    fn description(&self) -> &'static str { "파일을 '줄번호:내용' 형식으로 반환 (edit.replace_lines로 줄 지정 편집 시 근거로 사용)" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})
    }
    fn category(&self, _args: &Value) -> Category { Category::Read }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        let s = std::fs::read_to_string(&path).map_err(|e| ToolError::Message(format!("읽기 실패 {path}: {e}")))?;
        const MAX_LINES: usize = 2000;
        let lines: Vec<&str> = s.lines().collect();
        let take = lines.len().min(MAX_LINES);
        let body = lines[..take].iter().enumerate().map(|(i, l)| format!("{}:{}", i + 1, l)).collect::<Vec<_>>().join("\n");
        if lines.len() > take { Ok(format!("{body}\n…({take}줄 이후 생략, 전체 {}줄)", lines.len())) } else { Ok(body) }
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

pub struct FsMkdir;
impl Tool for FsMkdir {
    fn name(&self) -> &'static str { "fs.mkdir" }
    fn description(&self) -> &'static str { "디렉터리를 생성한다 (중간 경로 포함, 이미 있으면 통과)" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})
    }
    fn category(&self, _args: &Value) -> Category { Category::Write }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        std::fs::create_dir_all(&path).map_err(|e| ToolError::Message(format!("디렉터리 생성 실패 {path}: {e}")))?;
        Ok(format!("{path} 디렉터리 준비 완료"))
    }
}

pub struct FsMove;
impl Tool for FsMove {
    fn name(&self) -> &'static str { "fs.move" }
    fn description(&self) -> &'static str { "파일/디렉터리를 dest 경로로 이동한다" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"dest":{"type":"string"}},"required":["path","dest"]})
    }
    fn category(&self, _args: &Value) -> Category { Category::Write }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        let dest = arg_str(args, "dest")?;
        if let Some(dir) = std::path::Path::new(&dest).parent() { std::fs::create_dir_all(dir)?; } // fs.write 관례와 동일
        std::fs::rename(&path, &dest).map_err(|e| ToolError::Message(format!("이동 실패 {path} → {dest}: {e}")))?;
        Ok(format!("{path} → {dest} 이동 완료"))
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

/// 줄번호 인자 파싱 — LLM이 2.0처럼 정수를 실수로 보내도 수용
fn arg_line_no(args: &Value, key: &str) -> Result<usize, ToolError> {
    let v = args.get(key).ok_or_else(|| ToolError::Message(format!("{key} 인자 누락")))?;
    let n = v.as_u64()
        .or_else(|| v.as_f64().filter(|f| *f >= 0.0 && f.fract() == 0.0).map(|f| f as u64))
        .ok_or_else(|| ToolError::Message(format!("{key} 인자는 정수여야 함")))?;
    Ok(n as usize)
}

pub struct EditReplaceLines;
impl Tool for EditReplaceLines {
    fn name(&self) -> &'static str { "edit.replace_lines" }
    fn description(&self) -> &'static str { "start~end 줄(1-based, end 포함)을 content 줄들로 교체한다 — 빈 content면 해당 줄 삭제, fs.read_lines 출력의 줄번호를 그대로 쓴다" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"path":{"type":"string"},"start":{"type":"integer"},"end":{"type":"integer"},"content":{"type":"string"}},"required":["path","start","end","content"]})
    }
    fn category(&self, _args: &Value) -> Category { Category::Write }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let path = arg_str(args, "path")?;
        let start = arg_line_no(args, "start")?;
        let end = arg_line_no(args, "end")?;
        let content = arg_str(args, "content")?;
        let s = std::fs::read_to_string(&path).map_err(|e| ToolError::Message(format!("읽기 실패 {path}: {e}")))?;
        let had_trailing_nl = s.ends_with('\n');
        let mut lines: Vec<String> = s.lines().map(String::from).collect();
        let total = lines.len();
        if start == 0 || start > end || end > total {
            return Err(ToolError::Message(format!("잘못된 줄 범위 {start}~{end} (전체 {total}줄, 1-based)")));
        }
        let repl: Vec<String> = content.lines().map(String::from).collect(); // 빈 content → 빈 벡터 = 삭제
        lines.splice(start - 1..end, repl);
        let mut out = lines.join("\n");
        if had_trailing_nl && !out.is_empty() { out.push('\n'); } // 개행 관례 보존
        std::fs::write(&path, &out).map_err(|e| ToolError::Message(format!("쓰기 실패 {path}: {e}")))?;
        Ok(format!("{start}~{end}줄 교체 완료 ({total}줄 → {}줄)", lines.len()))
    }
}
