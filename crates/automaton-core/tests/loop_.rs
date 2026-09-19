use automaton_core::*;
use automaton_policy::Engine;
use automaton_proto::{ActionInfo, Event, Mode};
use automaton_tools::Registry;
use serde_json::json;
use parking_lot::Mutex;
use futures_util::StreamExt;

struct Scripted { turns: Mutex<Vec<Vec<StreamItem>>>, call: Mutex<usize> }
#[async_trait::async_trait]
impl Provider for Scripted {
    async fn complete(&self, _req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError> {
        let mut i = self.call.lock();
        let t = self.turns.lock();
        let items = t.get(*i).cloned().unwrap_or_default();
        *i += 1; // 턴 인덱스 전진 — 누락 시 모든 complete()가 turn 0 반환 (실측 결함 방지)
        Ok(items)
    }
}

/// complete_stream 오버라이드가 Box<dyn>/&dyn 경로에서도 발동하는지 —
/// 전달 구현이 complete()만 위임하면 기본 버퍼링이 역행해 데몬 스트리밍이 죽는다 (실측 스모크 결함 회귀 가드).
struct Streaming { items: Vec<StreamItem> }
#[async_trait::async_trait]
impl Provider for Streaming {
    async fn complete(&self, _req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError> {
        panic!("complete_stream 오버라이드가 있으면 complete()는 불리지 않아야 함")
    }
    async fn complete_stream(&self, _req: CompletionRequest) -> Result<ProviderStream, CoreError> {
        Ok(futures_util::stream::iter(self.items.clone().into_iter().map(Ok)).boxed())
    }
}

struct AutoGate(ApprovalOutcome);
#[async_trait::async_trait]
impl ApprovalGate for AutoGate {
    async fn decide(&self, _session: &str, _a: ActionInfo) -> ApprovalOutcome { self.0.clone() }
}

fn tmp(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("automaton-core-{}/", std::process::id())).join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

async fn run(provider: Box<dyn Provider>, gate: Box<dyn ApprovalGate>, user: &str) -> Vec<Event> {
    let mut events = vec![];
    let lp = AgentLoop::new(provider, gate, Engine::builtin(), Registry::coding_set(), Mode::Code);
    let mut history = vec![];
    let mut emit = |e: Event| events.push(e);
    lp.run_turn("s1", &mut history, user.to_string(), &mut emit).await.unwrap();
    events
}

#[tokio::test]
async fn text_only_turn_streams_deltas() {
    let p = Scripted { turns: Mutex::new(vec![vec![StreamItem::Delta("안녕".into()), StreamItem::Delta("하세요".into())]]), call: Mutex::new(0) };

    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "인사해줘").await;
    assert!(ev.iter().filter(|e| matches!(e, Event::StreamDelta { .. })).count() >= 2);
    assert!(!ev.iter().any(|e| matches!(e, Event::ToolStarted { .. })));
}

#[tokio::test]
async fn usage_item_emits_usage_event_and_not_history() {
    let p = Scripted { turns: Mutex::new(vec![vec![
        StreamItem::Delta("답".into()),
        StreamItem::Usage(automaton_proto::TokenUsage { prompt_tokens: 7, completion_tokens: 3, total_tokens: 10 }),
    ]]), call: Mutex::new(0) };
    let evs = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "질문").await;
    assert!(evs.iter().any(|e| matches!(e, Event::Usage { usage, .. } if usage.total_tokens == 10)));
}

#[tokio::test]
async fn provider_stream_override_survives_dyn_forwarding() {
    let p = Streaming { items: vec![StreamItem::Delta("청크".into())] };
    // run()은 데몬과 동일하게 Box<dyn Provider>로 루프 구동 — 전달 누락 시 panic으로 실패
    let evs = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "질문").await;
    assert!(evs.iter().any(|e| matches!(e, Event::StreamDelta { delta, .. } if delta == "청크")));
}

#[tokio::test]
async fn allowed_tool_runs_and_result_recorded() {
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "fs.write".into(), args: json!({"path": tmp("a.txt"), "content": "brass"}) })],
            vec![StreamItem::Delta("기록했어".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "기록해").await;
    assert!(ev.iter().any(|e| matches!(e, Event::ToolStarted { tool, .. } if tool == "fs.write")));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: true, .. })));
    assert!(tmp("a.txt").exists());
}

#[tokio::test]
async fn destructive_tool_asks_then_executes_on_approval() {
    let f = tmp("del.txt");
    std::fs::write(&f, "x").unwrap();
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "fs.delete".into(), args: json!({"path": f}) })],
            vec![StreamItem::Delta("삭제 완료".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "삭제해").await;
    assert!(ev.iter().any(|e| matches!(e, Event::ApprovalRequested { .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: true, .. })));
    assert!(!f.exists());
}

#[tokio::test]
async fn denied_approval_leaves_file_and_reports_failure() {
    let f = tmp("keep.txt");
    std::fs::write(&f, "x").unwrap();
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "fs.delete".into(), args: json!({"path": f}) })],
            vec![StreamItem::Delta("취소됨".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Deny)), "삭제해").await;
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: false, .. })));
    assert!(f.exists());
}

#[tokio::test]
async fn policy_deny_skips_gate_entirely() {
    // 등록된 툴(fs.read)이지만 타깃 경로에 Password가 포함돼 정책 1단계 하드 거부 경로를 탄다.
    // (레지스트리에 없는 툴을 쓰면 unknown-tool 경로와 구분되지 않아 이 테스트가 무의미해짐)
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "fs.read".into(), args: json!({"path": tmp("Password.kdbx")}) })],
            vec![StreamItem::Delta("못 읽었어".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "비밀번호 파일 읽어줘").await;
    assert!(!ev.iter().any(|e| matches!(e, Event::ApprovalRequested { .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: false, .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { summary, .. } if summary.contains("민감 입력 필드"))));
}

#[tokio::test]
async fn unknown_tool_yields_error_result_and_loop_continues() {
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "없는툴".into(), args: json!({}) })],
            vec![StreamItem::Delta("계속".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "뭔가해줘").await;
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: false, .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::StreamDelta { delta, .. } if delta == "계속")));
}

#[tokio::test]
async fn runaway_provider_stops_at_max_turns() {
    // §9 가드: 비정상 프로바이더가 툴콜을 계속 반환해도 최대 턴 초과로 중단
    let turns: Vec<Vec<StreamItem>> = (0..40).map(|_| vec![StreamItem::ToolCall(ToolCall { name: "fs.read".into(), args: json!({"path": "Cargo.toml"}) })]).collect();
    let p = Scripted { turns: Mutex::new(turns), call: Mutex::new(0) };
    let lp = AgentLoop::new(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), Engine::builtin(), Registry::coding_set(), Mode::Code);
    let mut history = vec![];
    let r = lp.run_turn("s1", &mut history, "계속해".into(), &mut |_| {}).await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("최대 턴"));
}

#[tokio::test]
async fn triple_consecutive_failures_abort_turn() {
    // §9 가드: 동일 툴 연속 3회 실패 시 중단
    let t = || vec![StreamItem::ToolCall(ToolCall { name: "없는툴".into(), args: json!({}) })];
    let p = Scripted { turns: Mutex::new(vec![t(), t(), t(), vec![StreamItem::Delta("x".into())]]), call: Mutex::new(0) };
    let lp = AgentLoop::new(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), Engine::builtin(), Registry::coding_set(), Mode::Code);
    let mut history = vec![];
    let r = lp.run_turn("s1", &mut history, "고장".into(), &mut |_| {}).await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("연속 3회 실패"));
}

#[tokio::test]
async fn decoy_target_does_not_bypass_sensitive_path_deny() {
    // 적대적 모델이 decoy target: "benign.txt"를 보내도 실제 path "Password.kdbx"가 검사되어 Deny되어야 함
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "fs.read".into(), args: json!({"path": tmp("Password.kdbx"), "target": "benign.txt"}) })],
            vec![StreamItem::Delta("거부됨 확인".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "비밀번호 파일 읽어줘").await;
    assert!(!ev.iter().any(|e| matches!(e, Event::ApprovalRequested { .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: false, .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { summary, .. } if summary.contains("민감 입력 필드"))));
}

#[tokio::test]
async fn shell_exec_with_decoy_path_does_not_mask_sensitive_command() {
    // 적대적 모델이 shell.exec에 decoy path: "notes.txt"를 보내도 실제 command 안의 Password가 검사되어 Deny되어야 함
    let p = Scripted {
        turns: Mutex::new(vec![
            vec![StreamItem::ToolCall(ToolCall { name: "shell.exec".into(), args: json!({"command": "cat ~/Passwords.kdbx", "path": "notes.txt"}) })],
            vec![StreamItem::Delta("거부됨 확인".into())],
        ]),
        call: Mutex::new(0),
    };
    let ev = run(Box::new(p), Box::new(AutoGate(ApprovalOutcome::Approve)), "비밀번호 출력해줘").await;
    assert!(!ev.iter().any(|e| matches!(e, Event::ApprovalRequested { .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { ok: false, .. })));
    assert!(ev.iter().any(|e| matches!(e, Event::ToolResult { summary, .. } if summary.contains("민감 입력 필드"))));
}
