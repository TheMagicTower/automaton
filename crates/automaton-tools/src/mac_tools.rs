//! mac 툴 (§5 mac 모드 툴셋). M1 구현 선택:
//! - capture.screen: 시스템 `screencapture` CLI 브리지 (파일 저장 후 경로 반환)
//! - ax.read: `osascript -l JavaScript` 브리지로 최전면 앱·윈도우 제목 JSON 반환
//! - input.click / input.type: core-graphics CGEvent (type은 pbcopy+Cmd+V — 한글 등 유니코드 지원, 클립보드 덮어씀 주의)

use crate::{Tool, ToolError};
use automaton_policy::Category;
use core_graphics::event::{CGEvent, CGEventTapLocation, CGMouseButton};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use serde_json::Value;

pub struct CaptureScreen;
impl Tool for CaptureScreen {
    fn name(&self) -> &'static str { "capture.screen" }
    fn description(&self) -> &'static str { "화면을 캡처해 png 파일 경로를 반환" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"path":{"type":"string"}}})
    }
    fn category(&self, _args: &Value) -> Category { Category::Read }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let path = args.get("path").and_then(|v| v.as_str()).map(String::from)
            .unwrap_or_else(|| std::env::temp_dir().join(format!("automaton-capture-{}.png", std::process::id())).to_string_lossy().into());
        let out = std::process::Command::new("screencapture").arg("-x").arg(&path).output()
            .map_err(|e| ToolError::Message(format!("screencapture 실행 실패: {e}")))?;
        if !out.status.success() {
            return Err(ToolError::Message(format!("캡처 실패(exit {}): 스크린 레코딩 권한 확인 필요", out.status)));
        }
        Ok(path)
    }
}

#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct AxSummary { pub app: String, pub title: String }

pub struct AxRead;
impl AxRead {
    pub fn parse_summary(&self, raw: &str) -> Result<AxSummary, ToolError> {
        serde_json::from_str(raw).map_err(|e| ToolError::Message(format!("AX 요약 파싱 실패: {e}")))
    }
}
impl Tool for AxRead {
    fn name(&self) -> &'static str { "ax.read" }
    fn description(&self) -> &'static str { "최전면 앱·윈도우 제목 요약을 반환" }
    fn parameters_schema(&self) -> Value { serde_json::json!({"type":"object","properties":{}}) }
    fn category(&self, _args: &Value) -> Category { Category::Read }
    fn execute(&self, _args: &Value) -> Result<String, ToolError> {
        let script = r#"
            ObjC.import('Foundation');
            const se = Application('System Events');
            const p = se.processes.whose({frontmost: true})[0];
            const title = p.windows.length > 0 ? p.windows[0].name() : '';
            JSON.stringify({app: p.name(), title: title});
        "#;
        let out = std::process::Command::new("osascript").arg("-l").arg("JavaScript").arg("-e").arg(script).output()
            .map_err(|e| ToolError::Message(format!("osascript 실행 실패: {e}")))?;
        if !out.status.success() {
            return Err(ToolError::Message(format!("AX 읽기 실패: 접근성 권한 확인 필요 ({})", String::from_utf8_lossy(&out.stderr))));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

pub struct InputClick;
impl Tool for InputClick {
    fn name(&self) -> &'static str { "input.click" }
    fn description(&self) -> &'static str { "좌표 (x,y)를 좌클릭" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"},"app":{"type":"string"}},"required":["x","y"]})
    }
    fn category(&self, _args: &Value) -> Category { Category::Input }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let x = args.get("x").and_then(|v| v.as_f64()).ok_or_else(|| ToolError::Message("x 인자 누락".into()))?;
        let y = args.get("y").and_then(|v| v.as_f64()).ok_or_else(|| ToolError::Message("y 인자 누락".into()))?;
        let pt = CGPoint { x, y };
        let src = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
            .map_err(|_| ToolError::Message("CGEventSource 생성 실패".into()))?;
        for down in [true, false] {
            let kind = if down { core_graphics::event::CGEventType::LeftMouseDown } else { core_graphics::event::CGEventType::LeftMouseUp };
            let ev = CGEvent::new_mouse_event(src.clone(), kind, pt, CGMouseButton::Left)
                .map_err(|_| ToolError::Message("이벤트 생성 실패".into()))?;
            ev.post(CGEventTapLocation::HID);
        }
        Ok(format!("({x},{y}) 클릭 완료"))
    }
}

pub struct InputType;
impl Tool for InputType {
    fn name(&self) -> &'static str { "input.type" }
    fn description(&self) -> &'static str { "클립보드 붙여넣기로 텍스트 입력 (한글 지원, 클립보드 덮어씀)" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"text":{"type":"string"},"app":{"type":"string"}},"required":["text"]})
    }
    fn category(&self, _args: &Value) -> Category { Category::Input }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let text = args.get("text").and_then(|v| v.as_str()).ok_or_else(|| ToolError::Message("text 인자 누락".into()))?;
        // 1) 클립보드에 텍스트 적재
        let mut child = std::process::Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn()
            .map_err(|e| ToolError::Message(format!("pbcopy 실행 실패: {e}")))?;
        use std::io::Write;
        child.stdin.take().unwrap().write_all(text.as_bytes()).map_err(|e| ToolError::Message(format!("pbcopy 쓰기 실패: {e}")))?;
        child.wait().map_err(|e| ToolError::Message(format!("pbcopy 대기 실패: {e}")))?;
        // 2) Cmd+V 가상키 (V=9, CGKeyCode=u16) — CGEvent 생성 실패는 () 오류라 map_err(|_| ...)로 통일
        const V_KEY: core_graphics::event::CGKeyCode = 9;
        let src = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
            .map_err(|_| ToolError::Message("CGEventSource 생성 실패".into()))?;
        for down in [true, false] {
            let ev = CGEvent::new_keyboard_event(src.clone(), V_KEY, down)
                .map_err(|_| ToolError::Message("키 이벤트 생성 실패".into()))?;
            ev.set_flags(core_graphics::event::CGEventFlags::CGEventFlagCommand);
            ev.post(CGEventTapLocation::HID);
        }
        Ok(format!("\"{text}\" 입력 완료 (붙여넣기 방식)"))
    }
}
