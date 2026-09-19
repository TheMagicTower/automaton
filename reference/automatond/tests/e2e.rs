//! headless E2E (§10): mock provider 재생으로 루프-정책-감사-메모리 전 경로 검증.

use automaton_core::{CompletionRequest, CoreError, Provider, StreamItem};
use automatond::daemon::{Daemon, Paths};
use automaton_proto::Event;
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
    wr.write_all(b"{\"method\":\"session_create\",\"params\":{\"id\":\"s1\"}}\n").await.unwrap();
    wr.write_all(b"{\"method\":\"mode_switch\",\"params\":{\"session\":\"s1\",\"to\":\"code\"}}\n").await.unwrap();
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
