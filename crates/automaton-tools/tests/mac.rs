use automaton_policy::Category;
use automaton_tools::{
    AxListElements, AxRead, CaptureScreen, InputClick, InputClickElement, InputType, Tool,
};
use serde_json::json;

#[test]
fn mac_tool_categories_match_spec() {
    assert_eq!(CaptureScreen.category(&json!({})), Category::Read);
    assert_eq!(AxRead.category(&json!({})), Category::Read);
    assert_eq!(
        InputClick.category(&json!({"x": 1, "y": 2, "target": "button 'OK'"})),
        Category::Input
    );
    assert_eq!(
        InputType.category(&json!({"text": "hi", "target": "search field"})),
        Category::Input
    );
}

#[test]
fn input_tools_without_target_classify_external_f04() {
    // F-04 회귀: target은 모델 자기선언 — 누락·빈 값이면 민감 필드(SecureTextField) 검사가
    // 우회되므로 Input이 아닌 External(항상 ASK)로 분류.
    assert_eq!(
        InputClick.category(&json!({"x": 1, "y": 2})),
        Category::External
    );
    assert_eq!(
        InputType.category(&json!({"text": "hi"})),
        Category::External
    );
    assert_eq!(
        InputType.category(&json!({"text": "hi", "target": "   "})),
        Category::External
    );
    assert_eq!(
        InputClick.category(&json!({"x": 1, "y": 2, "target": 42})),
        Category::External
    ); // 비문자열 target
}

#[test]
fn input_tools_require_args() {
    assert!(InputClick.execute(&json!({})).is_err()); // x/y 누락
    assert!(InputType.execute(&json!({})).is_err()); // text 누락
}

#[test]
fn capture_screen_schema_allows_no_path_injection() {
    assert_eq!(
        CaptureScreen.parameters_schema(),
        json!({"type":"object","properties":{}})
    );
}

#[test]
fn ax_summary_parses_frontmost_json() {
    let s = AxRead
        .parse_summary(r#"{"app":"Finder","title":"Downloads"}"#)
        .unwrap();
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

#[test]
fn ax_list_elements_declares_read() {
    assert_eq!(AxListElements.category(&json!({})), Category::Read);
}

#[test]
fn ax_tree_parses_nested_element_json() {
    let raw = r#"{"role":"AXWindow","title":"기본","x":0,"y":0,"w":800,"h":600,"children":[{"role":"AXButton","title":"확인","x":10,"y":20,"w":80,"h":30,"truncated":true,"children":[]}]}"#;
    let tree = AxListElements.parse_tree(raw).unwrap();
    assert_eq!(tree.role, "AXWindow");
    assert_eq!(tree.x, Some(0.0));
    let btn = &tree.children.as_ref().unwrap()[0];
    assert_eq!(
        (btn.role.as_str(), btn.title.as_str()),
        ("AXButton", "확인")
    );
    assert_eq!(btn.x, Some(10.0));
    assert_eq!(btn.truncated, Some(true));
}

#[test]
fn input_click_element_classifies_by_target_f04() {
    assert_eq!(
        InputClickElement.category(&json!({"name":"확인","target":"button '확인'"})),
        Category::Input
    );
    assert_eq!(
        InputClickElement.category(&json!({"name":"확인"})),
        Category::External
    ); // target 누락
    assert_eq!(
        InputClickElement.category(&json!({"role":"AXButton","target":"   "})),
        Category::External
    ); // 빈 target
    assert_eq!(
        InputClickElement.category(&json!({"name":"x","target":7})),
        Category::External
    ); // 비문자열 target
}

#[test]
fn input_click_element_requires_search_operand() {
    // name/role 둘 다 없으면 정책 target이 있어도 실행 불가 — osascript 실행 전에 검증
    assert!(
        InputClickElement
            .execute(&json!({"target":"button"}))
            .is_err()
    );
}

#[test]
fn ax_hit_center_is_bbox_middle() {
    let hit = InputClickElement
        .parse_hit(r#"{"role":"AXButton","title":"OK","x":10,"y":20,"w":100,"h":50}"#)
        .unwrap();
    assert_eq!(hit.center(), Some((60.0, 45.0)));
    let bare = InputClickElement
        .parse_hit(r#"{"role":"AXCheckBox","x":5,"y":7}"#)
        .unwrap();
    assert_eq!(bare.center(), Some((5.0, 7.0))); // 크기 없으면 좌상단 그대로
    let no_pos = InputClickElement
        .parse_hit(r#"{"role":"AXButton"}"#)
        .unwrap();
    assert_eq!(no_pos.center(), None); // 좌표 없으면 클릭 불가
}

#[test]
fn mac_set_registers_element_tools() {
    let r = automaton_tools::Registry::mac_set();
    assert!(r.get("ax.list_elements").is_some());
    assert!(r.get("input.click_element").is_some());
}

#[test]
#[ignore = "접근성 권한 필요"]
fn lists_frontmost_element_tree() {
    let out = AxListElements.execute(&json!({})).unwrap();
    assert!(out.contains("\"role\""));
}

#[test]
#[ignore = "접근성·입력 이벤트 권한 필요 — 실제 화면을 클릭함"]
fn clicks_element_found_by_role() {
    let out = InputClickElement
        .execute(&json!({"role":"AXButton","target":"button"}))
        .unwrap();
    assert!(out.contains("클릭 완료"));
}
