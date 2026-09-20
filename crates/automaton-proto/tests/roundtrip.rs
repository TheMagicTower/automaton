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
fn draft_suggestions_event_roundtrip() {
    let ev = Event::DraftSuggestions { session: "s1".into(), suggestions: vec!["진행해".into(), "허용".into()] };
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains(r#""draft_suggestions""#));
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

#[test]
fn message_roundtrip() {
    let m = Message { role: "assistant".into(), content: "안녕하세요".into() };
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), m);
}

#[test]
fn usage_event_roundtrip() {
    let ev = Event::Usage {
        session: "s1".into(),
        usage: TokenUsage { prompt_tokens: 120, completion_tokens: 45, total_tokens: 165 },
    };
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains(r#""usage""#));
    assert!(json.contains(r#""prompt_tokens":120"#));
    assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), ev);
}

#[test]
fn memory_rpc_roundtrip() {
    // 요청 3종 — memory_browse / memory_delete / memory_stats (unit variant는 params 없이 직렬화)
    let req = Request::MemoryBrowse { offset: 10, limit: 50 };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""memory_browse""#));
    assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);

    let req = Request::MemoryDelete { content: "caspar는 한국어를 쓴다".into() };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""memory_delete""#));
    assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);

    let req = Request::MemoryStats;
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""memory_stats""#));
    assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);

    // 이벤트 — memory_data / memory_stats
    let ev = Event::MemoryData { facts: vec!["caspar는 한국어를 쓴다".into()], total: 1 };
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains(r#""memory_data""#));
    assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), ev);

    let ev = Event::MemoryStats { facts: 3, sessions: 2, decisions: 1 };
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains(r#""memory_stats""#));
    assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), ev);
}

#[test]
fn summary_rpc_roundtrip() {
    // 요청 — summary_get (사이드바 세션 요약 표시)
    let req = Request::SummaryGet { session: "shell-1".into() };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains(r#""summary_get""#));
    assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), req);

    // 이벤트 — summary_data (요약 없음 = 빈 문자열도 왕복)
    let ev = Event::SummaryData { session: "shell-1".into(), summary: "다운로드 폴더 정리를 마쳤다".into() };
    let json = serde_json::to_string(&ev).unwrap();
    assert!(json.contains(r#""summary_data""#));
    assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), ev);

    let ev = Event::SummaryData { session: "shell-2".into(), summary: String::new() };
    assert_eq!(serde_json::from_str::<Event>(&serde_json::to_string(&ev).unwrap()).unwrap(), ev);
}
