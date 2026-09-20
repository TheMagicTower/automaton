//! mac 툴 (§5 mac 모드 툴셋). M1 구현 선택:
//! - capture.screen: 시스템 `screencapture` CLI 브리지 (파일 저장 후 경로 반환)
//! - ax.read: `osascript -l JavaScript` 브리지로 최전면 앱·윈도우 제목 JSON 반환
//! - ax.list_elements: 최전면 앱 UI 요소 트리 JSON (JXA, 깊이 3 제한)
//! - input.click / input.type: core-graphics CGEvent (type은 pbcopy+Cmd+V — 한글 등 유니코드 지원, 클립보드 덮어씀 주의)
//! - input.click_element: JXA로 name/role 요소 검색 → 중심 좌표 CGEvent 클릭
//! - 민감 target 규약: target에 요소 역할(AxRole) 또는 이름을 포함 — 'SecureTextField'/'Password' 부분문자열은 정책 하드 거부 대상

use crate::{Tool, ToolError};
use automaton_policy::Category;
use core_graphics::event::{CGEvent, CGEventTapLocation, CGMouseButton};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;
use serde_json::Value;

pub struct CaptureScreen;
impl Tool for CaptureScreen {
    fn name(&self) -> &'static str {
        "capture.screen"
    }
    fn description(&self) -> &'static str {
        "화면을 캡처해 임시 png 파일 경로를 반환"
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    fn category(&self, _args: &Value) -> Category {
        Category::Read
    }
    fn execute(&self, _args: &Value) -> Result<String, ToolError> {
        // 임의 파일 덮어쓰기 원천 방지: 클라이언트 지정 경로를 허용하지 않고 항상 데몬 관리 임시 파일에만 기록
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let seq = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "automaton-capture-{}-{seq}.png",
            std::process::id()
        ));
        let path_str = path.to_string_lossy().to_string();
        let out = std::process::Command::new("screencapture")
            .arg("-x")
            .arg(&path_str)
            .output()
            .map_err(|e| ToolError::Message(format!("screencapture 실행 실패: {e}")))?;
        if !out.status.success() {
            return Err(ToolError::Message(format!(
                "캡처 실패(exit {}): 스크린 레코딩 권한 확인 필요",
                out.status
            )));
        }
        Ok(path_str)
    }
}

/// osascript JXA 공용 실행기 — 실패 시 접근성 권한 안내 포함
fn run_osascript(script: &str, err_context: &str) -> Result<String, ToolError> {
    let out = std::process::Command::new("osascript")
        .arg("-l")
        .arg("JavaScript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| ToolError::Message(format!("osascript 실행 실패: {e}")))?;
    if !out.status.success() {
        return Err(ToolError::Message(format!(
            "{err_context}: 접근성 권한 확인 필요 ({})",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// CGEvent 좌클릭(press+release) — input.click·input.click_element 공용
fn cg_click(x: f64, y: f64) -> Result<(), ToolError> {
    let pt = CGPoint { x, y };
    let src = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| ToolError::Message("CGEventSource 생성 실패".into()))?;
    for down in [true, false] {
        let kind = if down {
            core_graphics::event::CGEventType::LeftMouseDown
        } else {
            core_graphics::event::CGEventType::LeftMouseUp
        };
        let ev = CGEvent::new_mouse_event(src.clone(), kind, pt, CGMouseButton::Left)
            .map_err(|_| ToolError::Message("이벤트 생성 실패".into()))?;
        ev.post(CGEventTapLocation::HID);
    }
    Ok(())
}
#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct AxSummary {
    pub app: String,
    pub title: String,
}

pub struct AxRead;
impl AxRead {
    pub fn parse_summary(&self, raw: &str) -> Result<AxSummary, ToolError> {
        serde_json::from_str(raw).map_err(|e| ToolError::Message(format!("AX 요약 파싱 실패: {e}")))
    }
}
impl Tool for AxRead {
    fn name(&self) -> &'static str {
        "ax.read"
    }
    fn description(&self) -> &'static str {
        "최전면 앱·윈도우 제목 요약을 반환"
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    fn category(&self, _args: &Value) -> Category {
        Category::Read
    }
    fn execute(&self, _args: &Value) -> Result<String, ToolError> {
        let script = r#"
            ObjC.import('Foundation');
            const se = Application('System Events');
            const p = se.processes.whose({frontmost: true})[0];
            const title = p.windows.length > 0 ? p.windows[0].name() : '';
            JSON.stringify({app: p.name(), title: title});
        "#;
        run_osascript(script, "AX 읽기 실패")
    }
}

pub struct InputClick;
impl Tool for InputClick {
    fn name(&self) -> &'static str {
        "input.click"
    }
    fn description(&self) -> &'static str {
        "좌표 (x,y)를 좌클릭 — 대상 요소의 AX 역할·이름을 target 인자로 전달해야 정책 엔진이 민감 필드(비밀번호 등)를 검사할 수 있다"
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"},"app":{"type":"string"},"target":{"type":"string","description":"대상 UI 요소의 역할/이름(예: button 'Delete', SecureTextField). 민감 입력 필드 식별에 필수"}},"required":["x","y"]})
    }
    fn category(&self, args: &Value) -> Category {
        // F-04: target은 모델 자기선언 — 누락하면 민감 필드(SecureTextField) 검사 자체가 생략된다.
        // target이 있을 때만 Input, 없으면 External(항상 ASK).
        args.get("target")
            .and_then(|v| v.as_str())
            .filter(|t| !t.trim().is_empty())
            .map(|_| Category::Input)
            .unwrap_or(Category::External)
    }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let x = args
            .get("x")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ToolError::Message("x 인자 누락".into()))?;
        let y = args
            .get("y")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| ToolError::Message("y 인자 누락".into()))?;
        cg_click(x, y)?;
        Ok(format!("({x},{y}) 클릭 완료"))
    }
}

pub struct InputType;
impl Tool for InputType {
    fn name(&self) -> &'static str {
        "input.type"
    }
    fn description(&self) -> &'static str {
        "클립보드 붙여넣기로 텍스트 입력 (한글 지원, 클립보드 덮어씀) — 대상 요소의 AX 역할·이름을 target 인자로 전달해야 정책 엔진이 민감 필드(비밀번호 등)를 검사할 수 있다"
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"text":{"type":"string"},"app":{"type":"string"},"target":{"type":"string","description":"대상 UI 요소의 역할/이름(예: SecureTextField, Password). 민감 입력 필드 식별에 필수"}},"required":["text"]})
    }
    fn category(&self, args: &Value) -> Category {
        // F-04: target은 모델 자기선언 — 누락하면 민감 필드(SecureTextField) 검사 자체가 생략된다.
        // target이 있을 때만 Input, 없으면 External(항상 ASK).
        args.get("target")
            .and_then(|v| v.as_str())
            .filter(|t| !t.trim().is_empty())
            .map(|_| Category::Input)
            .unwrap_or(Category::External)
    }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::Message("text 인자 누락".into()))?;
        // 1) 클립보드에 텍스트 적재
        let mut child = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ToolError::Message(format!("pbcopy 실행 실패: {e}")))?;
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .map_err(|e| ToolError::Message(format!("pbcopy 쓰기 실패: {e}")))?;
        child
            .wait()
            .map_err(|e| ToolError::Message(format!("pbcopy 대기 실패: {e}")))?;
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

/// AX UI 요소 노드 — ax.list_elements 트리·input.click_element 검색 결과의 공용 모양
#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct AxElement {
    pub role: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub w: Option<f64>,
    #[serde(default)]
    pub h: Option<f64>,
    #[serde(default)]
    pub truncated: Option<bool>,
    #[serde(default)]
    pub children: Option<Vec<AxElement>>,
}

pub struct AxListElements;
impl AxListElements {
    pub fn parse_tree(&self, raw: &str) -> Result<AxElement, ToolError> {
        serde_json::from_str(raw).map_err(|e| ToolError::Message(format!("AX 트리 파싱 실패: {e}")))
    }
}
impl Tool for AxListElements {
    fn name(&self) -> &'static str {
        "ax.list_elements"
    }
    fn description(&self) -> &'static str {
        "최전면 앱의 UI 요소 트리를 JSON으로 반환 (role·title·좌표·크기 포함, 깊이 3 제한)"
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"app":{"type":"string","description":"정책 평가용 앱 식별(선택)"}}})
    }
    fn category(&self, _args: &Value) -> Category {
        Category::Read
    }
    fn execute(&self, _args: &Value) -> Result<String, ToolError> {
        // System Events JXA 탐색: 최전면 프로세스의 첫 창을 루트로 깊이 3까지 수집.
        // position/size는 배열·{x,y}·{width,height} 어느 형태로 와도 [a,b]로 정규화.
        let script = r#"
            ObjC.import('Foundation');
            const se = Application('System Events');
            const p = se.processes.whose({frontmost: true})[0];
            function pt2(v) {
                if (!v) return null;
                if (Array.isArray(v)) return [v[0], v[1]];
                if (v.x !== undefined) return [v.x, v.y];
                if (v.width !== undefined) return [v.width, v.height];
                return null;
            }
            function el(e, depth) {
                const o = {role: '', title: '', children: []};
                try { o.role = e.role() || ''; } catch (_) {}
                try { o.title = e.title() || ''; } catch (_) {}
                if (!o.title) { try { o.title = e.description() || ''; } catch (_) {} }
                try { const q = pt2(e.position()); if (q) { o.x = q[0]; o.y = q[1]; } } catch (_) {}
                try { const s = pt2(e.size()); if (s) { o.w = s[0]; o.h = s[1]; } } catch (_) {}
                if (depth < 3) {
                    let kids = [];
                    try { kids = e.uiElements(); } catch (_) {}
                    for (let i = 0; i < kids.length; i++) o.children.push(el(kids[i], depth + 1));
                } else { o.truncated = true; }
                return o;
            }
            let root = null;
            if (p.windows.length > 0) root = el(p.windows[0], 1);
            JSON.stringify(root);
        "#;
        let out = run_osascript(script, "AX 요소 트리 읽기 실패")?;
        if out == "null" {
            return Err(ToolError::Message(
                "최전면 앱에 접근 가능한 창이 없음".into(),
            ));
        }
        Ok(out)
    }
}

/// input.click_element의 JXA 검색 결과 — 중심 좌표 계산 포함
#[derive(serde::Deserialize, Debug, PartialEq)]
pub struct AxHit {
    pub role: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub w: Option<f64>,
    #[serde(default)]
    pub h: Option<f64>,
}

impl AxHit {
    /// 요소 중심 좌표 — x/y가 없으면 클릭 불가 (None). 크기가 없으면 좌상단.
    pub fn center(&self) -> Option<(f64, f64)> {
        let (x, y) = (self.x?, self.y?);
        let (dx, dy) = match (self.w, self.h) {
            (Some(w), Some(h)) => (w / 2.0, h / 2.0),
            _ => (0.0, 0.0),
        };
        Some((x + dx, y + dy))
    }
}

pub struct InputClickElement;
impl InputClickElement {
    pub fn parse_hit(&self, raw: &str) -> Result<AxHit, ToolError> {
        serde_json::from_str(raw)
            .map_err(|e| ToolError::Message(format!("AX 검색 결과 파싱 실패: {e}")))
    }
}
impl Tool for InputClickElement {
    fn name(&self) -> &'static str {
        "input.click_element"
    }
    fn description(&self) -> &'static str {
        "최전면 앱에서 name(제목/설명 부분일치) 또는 role(정확일치)로 UI 요소를 찾아 중심 좌표를 클릭 — 대상 요소의 AX 역할·이름을 target 인자로 전달해야 정책 엔진이 민감 필드(비밀번호 등)를 검사할 수 있다"
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"name":{"type":"string","description":"요소 제목/설명의 부분문자열"},"role":{"type":"string","description":"AXRole 정확 일치 (예: AXButton)"},"app":{"type":"string"},"target":{"type":"string","description":"대상 UI 요소의 역할/이름(예: button 'Delete', SecureTextField). 민감 입력 필드 식별에 필수"}},"required":["target"]})
    }
    fn category(&self, args: &Value) -> Category {
        // F-04: target은 모델 자기선언 — 누락·빈 값이면 민감 필드(SecureTextField) 검사가
        // 우회되므로 Input이 아닌 External(항상 ASK)로 분류.
        args.get("target")
            .and_then(|v| v.as_str())
            .filter(|t| !t.trim().is_empty())
            .map(|_| Category::Input)
            .unwrap_or(Category::External)
    }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let role = args.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if name.trim().is_empty() && role.trim().is_empty() {
            return Err(ToolError::Message("name 또는 role 검색 조건이 필요".into()));
        }
        // JSON 문자열 리터럴은 그대로 유효한 JS 리터럴 — 안전하게 스크립트에 주입
        let encode = |s: &str| {
            serde_json::to_string(s)
                .map_err(|e| ToolError::Message(format!("인자 인코딩 실패: {e}")))
        };
        let script = r#"
            ObjC.import('Foundation');
            const se = Application('System Events');
            const p = se.processes.whose({frontmost: true})[0];
            const NAME = @NAME@;
            const ROLE = @ROLE@;
            function pt2(v) {
                if (!v) return null;
                if (Array.isArray(v)) return [v[0], v[1]];
                if (v.x !== undefined) return [v.x, v.y];
                if (v.width !== undefined) return [v.width, v.height];
                return null;
            }
            function walk(e, depth) {
                if (depth > 6) return null;
                let role = '', title = '', desc = '';
                try { role = e.role() || ''; } catch (_) {}
                try { title = e.title() || ''; } catch (_) {}
                try { desc = e.description() || ''; } catch (_) {}
                const nameOk = NAME === '' || title.indexOf(NAME) >= 0 || desc.indexOf(NAME) >= 0;
                const roleOk = ROLE === '' || role === ROLE;
                if (nameOk && roleOk) {
                    const o = {role: role, title: title || desc, x: null, y: null, w: null, h: null};
                    try { const q = pt2(e.position()); if (q) { o.x = q[0]; o.y = q[1]; } } catch (_) {}
                    try { const s = pt2(e.size()); if (s) { o.w = s[0]; o.h = s[1]; } } catch (_) {}
                    return o;
                }
                let kids = [];
                try { kids = e.uiElements(); } catch (_) {}
                for (let i = 0; i < kids.length; i++) { const r = walk(kids[i], depth + 1); if (r) return r; }
                return null;
            }
            let hit = null;
            if (p.windows.length > 0) hit = walk(p.windows[0], 1);
            JSON.stringify(hit);
        "#
        .replace("@NAME@", &encode(name)?)
        .replace("@ROLE@", &encode(role)?);
        let raw = run_osascript(&script, "요소 검색 실패")?;
        if raw == "null" {
            return Err(ToolError::Message(format!(
                "요소를 찾지 못함 (name='{name}', role='{role}')"
            )));
        }
        let hit = self.parse_hit(&raw)?;
        let (x, y) = hit
            .center()
            .ok_or_else(|| ToolError::Message(format!("요소에 좌표가 없음: '{}'", hit.title)))?;
        cg_click(x, y)?;
        Ok(format!(
            "({x:.0},{y:.0}) '{}'({}) 클릭 완료",
            hit.title, hit.role
        ))
    }
}
