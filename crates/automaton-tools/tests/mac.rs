use automaton_policy::Category;
use automaton_tools::{AxRead, CaptureScreen, InputClick, InputType, Tool};
use serde_json::json;

#[test]
fn mac_tool_categories_match_spec() {
    assert_eq!(CaptureScreen.category(&json!({})), Category::Read);
    assert_eq!(AxRead.category(&json!({})), Category::Read);
    assert_eq!(InputClick.category(&json!({"x": 1, "y": 2})), Category::Input);
    assert_eq!(InputType.category(&json!({"text": "hi"})), Category::Input);
}

#[test]
fn input_tools_require_args() {
    assert!(InputClick.execute(&json!({})).is_err());   // x/y 누락
    assert!(InputType.execute(&json!({})).is_err());    // text 누락
}

#[test]
fn capture_screen_schema_allows_no_path_injection() {
    assert_eq!(CaptureScreen.parameters_schema(), json!({"type":"object","properties":{}}));
}

#[test]
fn ax_summary_parses_frontmost_json() {
    let s = AxRead.parse_summary(r#"{"app":"Finder","title":"Downloads"}"#).unwrap();
    assert_eq!(s.app, "Finder");
    assert_eq!(s.title, "Downloads");
}

#[test]
fn mac_set_registers_four_tools() {
    let r = automaton_tools::Registry::mac_set();
    assert!(r.get("capture.screen").is_some());
    assert!(r.get("ax.read").is_some());
    assert!(r.get("input.click").is_some());
    assert!(r.get("input.type").is_some());
}

// 권한 필요 — 로컬 수동 실행 전용: cargo test -- --ignored
#[test]
#[ignore = "스크린 레코딩 권한 필요"]
fn captures_screen_to_file() {
    let out = CaptureScreen.execute(&json!({})).unwrap();
    assert!(out.contains(".png"));
}

#[test]
#[ignore = "접근성 권한 필요"]
fn reads_frontmost_summary() {
    let out = AxRead.execute(&json!({})).unwrap();
    assert!(out.contains("app"));
}
