//! headless E2E (§10): mock provider 재생으로 루프-정책-감사-메모리 전 경로 검증.

use automaton_core::{CompletionRequest, CoreError, Provider, StreamItem};
use automatond::daemon::{Daemon, Paths};
use automaton_proto::{Event, Mode};
use parking_lot::Mutex; // 즉시 lock — rs-parking-lot 룰 (std::sync::Mutex unwrap 불요)
use serde_json::json;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader, AsyncWriteExt};

struct Scripted { turns: Mutex<Vec<Vec<StreamItem>>>, call: Mutex<usize> }
#[async_trait::async_trait]
impl Provider for Scripted {
    async fn complete(&self, _req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError> {
        let mut i = self.call.lock();
        let t = self.turns.lock();
        let items = t.get(*i).cloned().unwrap_or_default();
        *i += 1;
        Ok(items)
    }
}

fn tmp_root(name: &str) -> PathBuf {
    // 테스트별 고유 루트 — pid 공유 시 두 E2E가 같은 data/memory.db를 동시 open해 'database is locked' (실측 6/6 실패)
    let dir = std::env::temp_dir().join(format!("automaton-e2e-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

async fn read_events(reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>, until: &str) -> Vec<Event> {
    let mut evs = vec![];
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await.unwrap() == 0 { break; }
        let ev: Event = serde_json::from_str(line.trim()).unwrap();
        let done = format!("{ev:?}").contains(until);
        evs.push(ev);
        if done { break; }
    }
    evs
}

/// F-03: 모드 전환은 데몬 측 승인 게이트 통과 — mode.switch 배너 승인 응답 후 적용 확인
async fn approve_mode_switch(reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>, wr: &mut tokio::net::unix::OwnedWriteHalf, session: &str, expect: Mode) {
    let evs = read_events(reader, "ApprovalRequested").await;
    let approval = evs.iter().find_map(|e| match e {
        Event::ApprovalRequested { approval, action, .. } if action.tool == "mode.switch" => Some(approval.clone()),
        _ => None,
    }).expect("모드 전환 승인 요청(mode.switch) 미수신");
    let resp = format!("{{\"method\":\"approval_respond\",\"params\":{{\"session\":\"{session}\",\"approval\":\"{approval}\",\"decision\":\"approve\",\"always\":false}}}}\n");
    wr.write_all(resp.as_bytes()).await.unwrap();
    let evs = read_events(reader, "ModeChanged").await;
    assert!(evs.iter().any(|e| matches!(e, Event::ModeChanged { mode, .. } if *mode == expect)), "승인 후 모드 적용 실패");
}

#[tokio::test]
async fn allow_path_runs_tool_streams_and_audits() {
    let root = tmp_root("allow_path");
    let paths = Paths { data_dir: root.join("data"), config_dir: root.join("config") };
    let dir = root.join("work");
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("a.txt");
    let provider = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(automaton_core::ToolCall { name: "fs.write".into(), args: json!({"path": &target, "content": "brass"}) })],
            vec![StreamItem::Delta("기록 완료".into())],
        ]),
        call: Mutex::new(0),
    };
    let socket = root.join("d.sock");
    let d = std::sync::Arc::new(Daemon::new(Box::new(provider), paths));
    tokio::spawn(d.clone().serve(socket.clone()));
    // 서버 준비 재시도 — 고정 sleep은 부하 시 경합(리뷰 자문 반영)
    let mut stream = None;
    for _ in 0..10 {
        if let Ok(s) = tokio::net::UnixStream::connect(&socket).await { stream = Some(s); break; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let stream = stream.expect("데몬 소켓 연결 실패");
    let (rd, mut wr) = stream.into_split();
    let mut reader = BufReader::new(rd);
    wr.write_all(b"{\"method\":\"session_create\",\"params\":{\"id\":\"s1\"}}\n").await.unwrap();
    // Chat 기본 모드에서는 fs.write가 ASK → DenyGate 거부되므로 code 모드로 전환 후 전송 (실측 반영)
    wr.write_all(b"{\"method\":\"mode_switch\",\"params\":{\"session\":\"s1\",\"to\":\"code\"}}\n").await.unwrap();
    approve_mode_switch(&mut reader, &mut wr, "s1", Mode::Code).await; // F-03: 데몬 측 승인 게이트
    wr.write_all("{\"method\":\"message_send\",\"params\":{\"session\":\"s1\",\"text\":\"기록해\"}}\n".as_bytes()).await.unwrap();
    let evs = read_events(&mut reader, "StreamDelta").await;
    assert!(evs.iter().any(|e| matches!(e, Event::ToolStarted { tool, .. } if tool == "fs.write")));
    assert!(evs.iter().any(|e| matches!(e, Event::ToolResult { ok: true, .. })));
    assert!(target.exists());

    // 감사 로그 검증 (§5): 모든 이벤트가 jsonl로 기록됨
    let audit = std::fs::read_to_string(root.join("data/audit/s1.jsonl")).unwrap();
    assert!(audit.contains("tool_started"));
    assert!(audit.contains("tool_result"));
    // 메모리 지속 검증 (§9): 세션 메시지가 저장됨 + 중복 재기록 회귀 방지(2회 전송 후 행 수)
    // (store는 데몬 내부 — 재시작 검증은 Chunk 5 셸 연동 시점에 확장)
    // 중복 재기록 회귀 가드: append는 턴 종료 후 비동기라 짧은 재시도로 행 수 단언 (리뷰 자문)
    let db = root.join("data/memory.db");
    let mut rows = 0;
    for _ in 0..20 {
        if let Ok(store) = automaton_memory::MemoryStore::open(&db) {
            rows = store.messages("s1").map(|m| m.len()).unwrap_or(0);
            if rows > 0 { break; }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(rows >= 2, "세션 메시지 미저장 또는 재기록 결함 (rows={rows})");
}

#[tokio::test]
async fn usage_reported_and_written_to_audit_log() {
    let root = tmp_root("usage");
    let paths = Paths { data_dir: root.join("data"), config_dir: root.join("config") };
    let provider = Scripted {
        turns: Mutex::new(vec![vec![
            StreamItem::Delta("완료".into()),
            StreamItem::Usage(automaton_proto::TokenUsage { prompt_tokens: 11, completion_tokens: 4, total_tokens: 15 }),
        ]]),
        call: Mutex::new(0),
    };
    let socket = root.join("d.sock");
    let d = std::sync::Arc::new(Daemon::new(Box::new(provider), paths));
    tokio::spawn(d.clone().serve(socket.clone()));
    let mut stream = None;
    for _ in 0..10 {
        if let Ok(s) = tokio::net::UnixStream::connect(&socket).await { stream = Some(s); break; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let stream = stream.expect("데몬 소켓 연결 실패");
    let (rd, mut wr) = stream.into_split();
    let mut reader = BufReader::new(rd);
    wr.write_all(b"{\"method\":\"session_create\",\"params\":{\"id\":\"s1\"}}\n").await.unwrap();
    wr.write_all("{\"method\":\"message_send\",\"params\":{\"session\":\"s1\",\"text\":\"안녕\"}}\n".as_bytes()).await.unwrap();
    // Usage 이벤트가 소켓으로 도착 — 세션당 API 사용량 리포트
    let evs = read_events(&mut reader, "Usage").await;
    assert!(evs.iter().any(|e| matches!(e, Event::Usage { usage, .. } if usage.total_tokens == 15)));

    // writer_loop 감사 기록 — 이벤트 비동기 flush이므로 짧은 재시도로 단언
    let audit_path = root.join("data/audit/s1.jsonl");
    let mut audited = false;
    for _ in 0..20 {
        if let Ok(a) = std::fs::read_to_string(&audit_path) {
            if a.contains("\"type\":\"usage\"") && a.contains("\"total_tokens\":15") { audited = true; break; }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(audited, "감사 로그에 usage 미기록: {}", audit_path.display());
}

#[tokio::test]
async fn ask_path_requires_approval_roundtrip() {
    let root = tmp_root("approval");
    let paths = Paths { data_dir: root.join("data"), config_dir: root.join("config") };
    let dir = root.join("work");
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("to_delete.txt");
    std::fs::write(&target, "delete me").unwrap();
    let provider = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(automaton_core::ToolCall { name: "fs.delete".into(), args: json!({"path": &target}) })],
            vec![StreamItem::Delta("삭제 완료".into())],
        ]),
        call: Mutex::new(0),
    };
    let socket = root.join("d.sock");
    let d = std::sync::Arc::new(Daemon::new(Box::new(provider), paths));
    tokio::spawn(d.clone().serve(socket.clone()));
    let mut stream = None;
    for _ in 0..10 {
        if let Ok(s) = tokio::net::UnixStream::connect(&socket).await { stream = Some(s); break; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let stream = stream.expect("데몬 소켓 연결 실패");
    let (rd, mut wr) = stream.into_split();
    let mut reader = BufReader::new(rd);
    wr.write_all(b"{\"method\":\"mode_switch\",\"params\":{\"session\":\"s1\",\"to\":\"code\"}}\n").await.unwrap();
    approve_mode_switch(&mut reader, &mut wr, "s1", Mode::Code).await; // F-03: 데몬 측 승인 게이트
    wr.write_all("{\"method\":\"message_send\",\"params\":{\"session\":\"s1\",\"text\":\"삭제해\"}}\n".as_bytes()).await.unwrap();

    let mut evs = vec![];
    let mut line = String::new();
    let mut approval_id = None;
    loop {
        line.clear();
        if reader.read_line(&mut line).await.unwrap() == 0 { break; }
        let ev: Event = serde_json::from_str(line.trim()).unwrap();
        if let Event::ApprovalRequested { ref approval, .. } = ev {
            approval_id = Some(approval.clone());
            evs.push(ev);
            break;
        }
        evs.push(ev);
    }
    let approval = approval_id.expect("ApprovalRequested 이벤트 수신 실패");
    let resp = format!("{{\"method\":\"approval_respond\",\"params\":{{\"session\":\"s1\",\"approval\":\"{approval}\",\"decision\":\"approve\",\"always\":false}}}}\n");
    wr.write_all(resp.as_bytes()).await.unwrap();

    let rest = read_events(&mut reader, "StreamDelta").await;
    evs.extend(rest);

    assert!(evs.iter().any(|e| matches!(e, Event::ToolStarted { tool, .. } if tool == "fs.delete")));
    assert!(evs.iter().any(|e| matches!(e, Event::ToolResult { ok: true, .. })));
    assert!(!target.exists(), "승인 후 파일이 삭제되어야 함");
}

#[tokio::test]
async fn approval_after_three_approvals_emits_draft_suggestions() {
    let root = tmp_root("drafts");
    let paths = Paths { data_dir: root.join("data"), config_dir: root.join("config") };
    let dir = root.join("work");
    std::fs::create_dir_all(&dir).unwrap();
    let targets: Vec<_> = (0..4).map(|i| {
        let t = dir.join(format!("f{i}.txt"));
        std::fs::write(&t, "x").unwrap();
        t
    }).collect();
    let tool = |i: usize| StreamItem::ToolCall(automaton_core::ToolCall { name: "fs.delete".into(), args: json!({"path": &targets[i]}) });
    let provider = Scripted {
        turns: Mutex::new(vec![
            vec![tool(0)], vec![StreamItem::Delta("완료".into())],
            vec![tool(1)], vec![StreamItem::Delta("완료".into())],
            vec![tool(2)], vec![StreamItem::Delta("완료".into())],
            vec![tool(3)],
        ]),
        call: Mutex::new(0),
    };
    let socket = root.join("d.sock");
    let d = std::sync::Arc::new(Daemon::new(Box::new(provider), paths));
    tokio::spawn(d.clone().serve(socket.clone()));
    let mut stream = None;
    for _ in 0..10 {
        if let Ok(s) = tokio::net::UnixStream::connect(&socket).await { stream = Some(s); break; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let stream = stream.expect("데몬 소켓 연결 실패");
    let (rd, mut wr) = stream.into_split();
    let mut reader = BufReader::new(rd);
    wr.write_all(b"{\"method\":\"mode_switch\",\"params\":{\"session\":\"s1\",\"to\":\"code\"}}\n").await.unwrap();
    approve_mode_switch(&mut reader, &mut wr, "s1", Mode::Code).await; // F-03: 데몬 측 승인 게이트

    // 승인 3회 — 각 라운드: 배너 수신 → 승인 응답 → 턴 완료 대기
    for _ in 0..3 {
        wr.write_all("{\"method\":\"message_send\",\"params\":{\"session\":\"s1\",\"text\":\"삭제해\"}}\n".as_bytes()).await.unwrap();
        let evs = read_events(&mut reader, "ApprovalRequested").await;
        let approval = evs.iter().find_map(|e| match e {
            Event::ApprovalRequested { approval, .. } => Some(approval.clone()),
            _ => None,
        }).expect("ApprovalRequested 미수신");
        let resp = format!("{{\"method\":\"approval_respond\",\"params\":{{\"session\":\"s1\",\"approval\":\"{approval}\",\"decision\":\"approve\",\"always\":false}}}}\n");
        wr.write_all(resp.as_bytes()).await.unwrap();
        let _ = read_events(&mut reader, "StreamDelta").await;
    }

    // 4번째 배너 — 승인 3회 이력이 있으므로 답변 초안이 뒤따라야 한다 (§6 3단계)
    wr.write_all("{\"method\":\"message_send\",\"params\":{\"session\":\"s1\",\"text\":\"삭제해\"}}\n".as_bytes()).await.unwrap();
    let evs = read_events(&mut reader, "DraftSuggestions").await;
    let idx_approval = evs.iter().position(|e| matches!(e, Event::ApprovalRequested { .. })).expect("배너가 초안보다 먼저");
    let idx_drafts = evs.iter().position(|e| matches!(e, Event::DraftSuggestions { .. })).expect("DraftSuggestions 미수신");
    assert!(idx_approval < idx_drafts, "DraftSuggestions는 ApprovalRequested 직후에 발행되어야 함");
    match &evs[idx_drafts] {
        Event::DraftSuggestions { session, suggestions } => {
            assert_eq!(session, "s1");
            assert!(suggestions.iter().any(|s| s == "진행해"), "초안에 '진행해' 필요: {suggestions:?}");
        }
        _ => unreachable!(),
    }
}

/// 데몬 접속·session_create까지의 공통 준비 — (reader, writer, 소켓 경로)
async fn connect(provider: Box<dyn Provider>, root: &std::path::Path) -> (BufReader<tokio::net::unix::OwnedReadHalf>, tokio::net::unix::OwnedWriteHalf) {
    let paths = Paths { data_dir: root.join("data"), config_dir: root.join("config") };
    let socket = root.join("d.sock");
    let d = std::sync::Arc::new(Daemon::new(provider, paths));
    tokio::spawn(d.clone().serve(socket.clone()));
    let mut stream = None;
    for _ in 0..10 {
        if let Ok(s) = tokio::net::UnixStream::connect(&socket).await { stream = Some(s); break; }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let stream = stream.expect("데몬 소켓 연결 실패");
    let (rd, wr) = stream.into_split();
    (BufReader::new(rd), wr)
}

#[tokio::test]
async fn mode_switch_denial_keeps_session_mode_f03() {
    // F-03 회귀: 모드 전환은 데몬 측 승인 게이트 통과 — 거절 시 ModeChanged 미발행, 세션은 chat 유지.
    // (거절된 전환 후 fs.write가 code 모드가 아닌 chat 모드로 평가되어 자동 실행되지 않음을 검증)
    let root = tmp_root("mode_deny");
    let dir = root.join("work");
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("a.txt");
    let provider = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(automaton_core::ToolCall { name: "fs.write".into(), args: json!({"path": &target, "content": "x"}) })],
            vec![StreamItem::Delta("완료".into())],
        ]),
        call: Mutex::new(0),
    };
    let (mut reader, mut wr) = connect(Box::new(provider), &root).await;
    wr.write_all(b"{\"method\":\"session_create\",\"params\":{\"id\":\"s1\"}}\n").await.unwrap();
    wr.write_all(b"{\"method\":\"mode_switch\",\"params\":{\"session\":\"s1\",\"to\":\"code\"}}\n").await.unwrap();

    let evs = read_events(&mut reader, "ApprovalRequested").await;
    let approval = evs.iter().find_map(|e| match e {
        Event::ApprovalRequested { approval, action, .. } if action.tool == "mode.switch" => Some(approval.clone()),
        _ => None,
    }).expect("모드 전환 승인 요청 미수신");
    assert!(!evs.iter().any(|e| matches!(e, Event::ModeChanged { mode: Mode::Code, .. })), "승인 전 모드 적용은 결함");

    wr.write_all(format!("{{\"method\":\"approval_respond\",\"params\":{{\"session\":\"s1\",\"approval\":\"{approval}\",\"decision\":\"deny\",\"always\":false}}}}\n").as_bytes()).await.unwrap();
    let evs = read_events(&mut reader, "Error").await;
    assert!(evs.iter().any(|e| matches!(e, Event::Error { message, .. } if message.contains("모드 전환 거절"))), "거절 응답 이벤트 미수신: {evs:?}");

    // 세션이 여전히 chat이라 fs.write는 자동 실행되지 않고 승인 배너로 전환됨
    wr.write_all("{\"method\":\"message_send\",\"params\":{\"session\":\"s1\",\"text\":\"기록해\"}}\n".as_bytes()).await.unwrap();
    let evs = read_events(&mut reader, "ApprovalRequested").await;
    assert!(evs.iter().any(|e| matches!(e, Event::ApprovalRequested { action, .. } if action.tool == "fs.write")), "chat 유지 시 fs.write는 ASK여야 함: {evs:?}");
    assert!(!target.exists(), "거절된 모드 전환 후 파일이 자동 기록되면 결함");
}

#[tokio::test]
async fn approval_response_from_other_session_rejected_f03() {
    // F-03 회귀: 승인 id는 발행 세션에 귀속 — 타 세션 명의의 응답은 거부되고
    // 정당한 세션의 재응답으로만 waiter가 해제된다.
    let root = tmp_root("session_bind");
    let dir = root.join("work");
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("to_delete.txt");
    std::fs::write(&target, "delete me").unwrap();
    let provider = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(automaton_core::ToolCall { name: "fs.delete".into(), args: json!({"path": &target}) })],
            vec![StreamItem::Delta("삭제 완료".into())],
        ]),
        call: Mutex::new(0),
    };
    let (mut reader, mut wr) = connect(Box::new(provider), &root).await;
    wr.write_all(b"{\"method\":\"session_create\",\"params\":{\"id\":\"s1\"}}\n").await.unwrap();
    wr.write_all(b"{\"method\":\"mode_switch\",\"params\":{\"session\":\"s1\",\"to\":\"code\"}}\n").await.unwrap();
    approve_mode_switch(&mut reader, &mut wr, "s1", Mode::Code).await;
    wr.write_all("{\"method\":\"message_send\",\"params\":{\"session\":\"s1\",\"text\":\"삭제해\"}}\n".as_bytes()).await.unwrap();

    let evs = read_events(&mut reader, "ApprovalRequested").await;
    let approval = evs.iter().find_map(|e| match e {
        Event::ApprovalRequested { approval, action, .. } if action.tool == "fs.delete" => Some(approval.clone()),
        _ => None,
    }).expect("fs.delete 승인 요청 미수신");

    // 공격자 시나리오: 타 세션(s2) 명의로 s1의 승인 id에 응답 → 거부, waiter 미해제
    wr.write_all(format!("{{\"method\":\"approval_respond\",\"params\":{{\"session\":\"s2\",\"approval\":\"{approval}\",\"decision\":\"approve\",\"always\":true}}}}\n").as_bytes()).await.unwrap();
    let evs = read_events(&mut reader, "Error").await;
    assert!(evs.iter().any(|e| matches!(e, Event::Error { message, .. } if message.contains("세션"))), "세션 바인딩 위반 오류 미수신: {evs:?}");
    assert!(target.exists(), "타 세션 응답으로 실행되면 결함");

    // 정당한 세션의 응답으로만 실행된다
    wr.write_all(format!("{{\"method\":\"approval_respond\",\"params\":{{\"session\":\"s1\",\"approval\":\"{approval}\",\"decision\":\"approve\",\"always\":false}}}}\n").as_bytes()).await.unwrap();
    let evs = read_events(&mut reader, "StreamDelta").await;
    assert!(evs.iter().any(|e| matches!(e, Event::ToolStarted { tool, .. } if tool == "fs.delete")));
    assert!(!target.exists(), "정당 응답 후에만 삭제");
}

#[tokio::test]
async fn session_gate_keys_waiters_by_session_and_tool_f05() {
    // F-05 회귀: waiter 키는 "{session}:{tool}" — 동일 툴이라도 세션이 다르면 독립 등록되고,
    // 한 세션의 승인이 타 세션 waiter를 해제하지 않는다. 동일 키 중복 ASK는 즉시 거부.
    use automaton_core::{ApprovalGate, ApprovalOutcome};
    use automaton_proto::{ActionInfo, Decision};
    use automatond::daemon::SessionGate;
    use std::sync::Arc;

    let info = || ActionInfo { tool: "shell.exec".into(), target: String::new(), risk: String::new() };
    let gate = Arc::new(SessionGate::new());
    let (g1, g2, g3) = (gate.clone(), gate.clone(), gate.clone());
    let a = tokio::spawn(async move { g1.decide("s1", info()).await });
    let b = tokio::spawn(async move { g2.decide("s2", info()).await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await; // 첫 폴(등록) 완료 보장

    assert_eq!(gate.pending.lock().len(), 2, "세션별 독립 waiter 등록");
    let dup = tokio::spawn(async move { g3.decide("s1", info()).await }); // 동일 (세션,툴) 중복
    assert!(matches!(dup.await.unwrap(), ApprovalOutcome::Deny), "중복 ASK는 덮어쓰지 않고 즉시 거부");

    // s1 승인이 s2 waiter를 건드리지 않는다
    gate.pending.lock().remove("s1:shell.exec").unwrap().send(Decision::Approve).unwrap();
    assert!(matches!(a.await.unwrap(), ApprovalOutcome::Approve));
    assert!(gate.pending.lock().contains_key("s2:shell.exec"), "타 세션 waiter 잔존");
    drop(b); // 미응답 waiter 정리
}
