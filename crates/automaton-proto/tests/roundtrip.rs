use automaton_proto::*;

#[test]
fn request_roundtrip() {
    let req = Request::MessageSend { session: "s1".into(), text: "다운로드 정리해줘".into() };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""message_send""#));
    assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);
}

#[test]
fn approval_request_event_roundtrip() {
    let ev = Event::ApprovalRequested {
        session: "s1".into(),
        approval: "a1".into(),
        action: ActionInfo { tool: "fs.delete".into(), target: "~/Downloads/old.zip".into(), risk: "파일 1개 삭제".into() },
        hint: Some(Hint { text: "지난번 유사 상황에서 승인(3회)".into(), similar_count: 3 }),
    };
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains(r#""approval_requested""#));
    assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), ev);
}

#[test]
fn mode_serializes_lowercase() {
    assert_eq!(serde_json::to_string(&Mode::Mac).unwrap(), r#""mac""#);
}

#[test]
fn unknown_event_type_is_error_not_panic() {
    assert!(serde_json::from_str::<Event>(r#"{"type":"future_thing"}"#).is_err());
}
