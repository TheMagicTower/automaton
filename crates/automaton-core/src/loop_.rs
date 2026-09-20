//! 에이전트 루프 (§4·§9) — 스트리밍→툴콜→Policy→실행→결과 환류. 감사 로그 훅은 Chunk 4.

use crate::mode::ModeProfile;
use crate::provider::{CompletionRequest, CoreError, Message, Provider, ProviderStream, StreamItem, ToolCall};
use automaton_policy::{Action, Engine, Verdict};
use automaton_proto::{ActionInfo, Event, Mode};
use automaton_tools::{action_context, Registry, Tool};
use futures_util::StreamExt;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalOutcome { Approve, Deny }

/// ASK 판정 시 승인을 구하는 계약 — 데몬은 JSON-RPC 승인 배너로, 테스트는 자동응답으로 구현.
/// session 식별자 포함: 승인 대기열 키는 "{session}:{tool}" — 세션 간 승인 혼동 방지 (F-05).
#[async_trait::async_trait]
pub trait ApprovalGate: Send + Sync {
    async fn decide(&self, session: &str, action: ActionInfo) -> ApprovalOutcome;
}

// Box<dyn ...>로 감싼 트레이트 객체가 그대로 트레이트를 만족하도록 전달 구현 (테스트·데몬에서 Box 사용)
#[async_trait::async_trait]
impl<P: Provider + ?Sized> Provider for Box<P> {
    async fn complete(&self, req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError> { (**self).complete(req).await }
    // complete_stream도 위임 필수 — 기본 구현은 self.complete() 버퍼링이라 dyn 뒤 오버라이드에 못 닿음 (실측 스모크 결함)
    async fn complete_stream(&self, req: CompletionRequest) -> Result<ProviderStream, CoreError> { (**self).complete_stream(req).await }
}

#[async_trait::async_trait]
impl<G: ApprovalGate + ?Sized> ApprovalGate for Box<G> {
    async fn decide(&self, session: &str, action: ActionInfo) -> ApprovalOutcome { (**self).decide(session, action).await }
}

#[async_trait::async_trait]
impl<G: ApprovalGate + ?Sized> ApprovalGate for std::sync::Arc<G> {
    async fn decide(&self, session: &str, action: ActionInfo) -> ApprovalOutcome { (**self).decide(session, action).await }
}

// 참조(&dyn) 전달 구현 — 데몬이 Box<dyn Provider>를 참조로 넘길 때 필요 (Chunk 4 리뷰 반영)
#[async_trait::async_trait]
impl<P: Provider + ?Sized> Provider for &P {
    async fn complete(&self, req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError> { (**self).complete(req).await }
    async fn complete_stream(&self, req: CompletionRequest) -> Result<ProviderStream, CoreError> { (**self).complete_stream(req).await }
}

pub struct AgentLoop<P: Provider, G: ApprovalGate> {
    provider: P,
    gate: G,
    policy: Engine,
    registry: Registry,
    profile: ModeProfile,
    approval_seq: AtomicU32,
}

impl<P: Provider, G: ApprovalGate> AgentLoop<P, G> {
    pub fn with_profile(provider: P, gate: G, policy: Engine, registry: Registry, profile: ModeProfile) -> Self {
        AgentLoop { provider, gate, policy, registry, profile, approval_seq: AtomicU32::new(1) }
    }

    pub fn new(provider: P, gate: G, policy: Engine, registry: Registry, mode: Mode) -> Self {
        Self::with_profile(provider, gate, policy, registry, ModeProfile::builtin(mode))
    }

    pub async fn run_turn(&self, session: &str, history: &mut Vec<Message>, user: String, emit: &mut (dyn FnMut(Event) + Send)) -> Result<(), CoreError> {
        const MAX_TURNS: usize = 32; // 비정상 프로바이더 무한 반복 방지 (§9)
        let session = session.to_string(); // 데몬이 세션 id를 주입 (리뷰 반영 — 하드코딩 제거)
        history.push(Message { role: "user".into(), content: user });
        let mut consecutive_failures: usize = 0; // §9: 동일 툴 연속 실패 중단의 루프 수준 근사
        for _ in 0..MAX_TURNS {
            let req = self.build_request(history);
            // 청크 스트리밍 — 델타는 도착 즉시 emit (버퍼링 완성 대비 지연 제거)
            let mut stream: ProviderStream = self.provider.complete_stream(req).await?;
            let mut assistant = String::new();
            let mut calls = vec![];
            while let Some(item) = stream.next().await {
                match item? {
                    StreamItem::Delta(d) => { emit(Event::StreamDelta { session: session.clone(), delta: d.clone() }); assistant.push_str(&d); }
                    StreamItem::ToolCall(c) => calls.push(c),
                    StreamItem::Usage(u) => emit(Event::Usage { session: session.clone(), usage: u }),
                }
            }
            history.push(Message { role: "assistant".into(), content: assistant });
            if calls.is_empty() { return Ok(()); }
            for call in calls {
                let before = history.len();
                self.run_tool_call(&session, &call, history, emit).await?;
                let failed = matches!(history[before..].last(), Some(m) if m.content.contains("오류:") || m.content.contains("거부됨:") || m.content.contains("거절됨:"));
                consecutive_failures = if failed { consecutive_failures + 1 } else { 0 };
                if consecutive_failures >= 3 {
                    return Err(CoreError::Message("동일 툴 연속 3회 실패 — 중단 (§9)".into()));
                }
            }
        }
        Err(CoreError::Message(format!("최대 턴 {MAX_TURNS} 초과 — 중단")))
    }

    fn build_request(&self, history: &[Message]) -> CompletionRequest {
        let tools = self.profile.tools.iter().filter_map(|n| self.registry.get(n)).map(|t| (t.name().to_string(), t.description().to_string(), t.parameters_schema())).collect();
        CompletionRequest { system: self.profile.system_prompt.clone(), messages: history.to_vec(), tools, track_usage: true }
    }

    async fn run_tool_call(&self, session: &str, call: &ToolCall, history: &mut Vec<Message>, emit: &mut (dyn FnMut(Event) + Send)) -> Result<(), CoreError> {
        let Some(tool) = self.registry.get(&call.name) else {
            let msg = format!("알 수 없는 툴: {}", call.name);
            emit(Event::ToolResult { session: session.into(), tool: call.name.clone(), ok: false, summary: msg.clone() });
            history.push(Message { role: "tool".into(), content: format!("[{}] 오류: {msg}", call.name) });
            return Ok(());
        };
        let (app, target) = action_context(&call.name, &call.args);
        let action = Action { tool: call.name.clone(), category: tool.category(&call.args), app, target: target.clone() };
        let verdict = self.policy.evaluate(&action, self.profile.mode);
        let info = ActionInfo { tool: call.name.clone(), target: target.unwrap_or_default(), risk: summary_of(&action) };
        match verdict {
            Verdict::Allow => { self.execute_and_record(session, tool, call, history, emit); }
            Verdict::Ask { reason } => {
                let id = self.approval_seq.fetch_add(1, Ordering::SeqCst).to_string();
                emit(Event::ApprovalRequested { session: session.into(), approval: id, action: ActionInfo { risk: reason.clone(), ..info.clone() }, hint: None });
                let outcome = self.gate.decide(session, info).await;
                match outcome {
                    ApprovalOutcome::Approve => self.execute_and_record(session, tool, call, history, emit),
                    ApprovalOutcome::Deny => {
                        let msg = "사용자가 거절했습니다.".to_string();
                        emit(Event::ToolResult { session: session.into(), tool: call.name.clone(), ok: false, summary: msg.clone() });
                        history.push(Message { role: "tool".into(), content: format!("[{}] 거절됨: {msg}", call.name) });
                    }
                }
            }
            Verdict::Deny { reason } => {
                let msg = format!("정책 거부: {reason}");
                emit(Event::ToolResult { session: session.into(), tool: call.name.clone(), ok: false, summary: msg.clone() });
                history.push(Message { role: "tool".into(), content: format!("[{}] 거부됨: {msg}", call.name) });
            }
        }
        Ok(())
    }

    fn execute_and_record(&self, session: &str, tool: &dyn Tool, call: &ToolCall, history: &mut Vec<Message>, emit: &mut (dyn FnMut(Event) + Send)) {
        let summary = display_summary(&call.name, &call.args);
        emit(Event::ToolStarted { session: session.into(), tool: call.name.clone(), summary });
        let result = tool.execute(&call.args).map_err(|e| format!("오류: {e}"));
        let (ok, text) = match result { Ok(s) => (true, s), Err(e) => (false, e) };
        emit(Event::ToolResult { session: session.into(), tool: call.name.clone(), ok, summary: truncate(&text, 400) });
        history.push(Message { role: "tool".into(), content: format!("[{}] {}", call.name, text) });
    }
}

/// 툴 이름 + 실제 인자에서 사용자가 이해할 수 있는 요약 생성 — "shell.exec"만 보여주는 것이 아니라
/// "$ df -h" 또는 "📄 Cargo.toml"처럼 무엇을 하는지 즉시 알 수 있게.
fn display_summary(tool: &str, args: &serde_json::Value) -> String {
    let arg = |k: &str| args.get(k).and_then(|v| v.as_str()).unwrap_or("");
    match tool {
        "shell.exec" => format!("$ {}", arg("command")),
        "fs.read" | "fs.read_lines" => format!("📄 {}", arg("path")),
        "fs.write" => format!("📝 {} ({}B)", arg("path"), arg("content").len()),
        "fs.grep" => format!("🔍 \"{}\" in {}", arg("pattern"), arg("path")),
        "fs.delete" => format!("🗑 {}", arg("path")),
        "fs.mkdir" => format!("📁 {}", arg("path")),
        "fs.move" => format!("📦 {} → {}", arg("from"), arg("to")),
        "edit.apply" | "edit.replace_lines" => format!("✏️ {}", arg("path")),
        "capture.screen" => "📸 화면 캡처".into(),
        "ax.read" => "🔎 화면 요소 읽기".into(),
        "ax.list_elements" => "🌳 UI 요소 트리".into(),
        "input.click" => format!("👆 클릭 ({}, {})", arg("x"), arg("y")),
        "input.click_element" => format!("👆 클릭: {}", arg("element")),
        "input.type" => format!("⌨️ 입력: \"{}\"", truncate(arg("text"), 40)),
        _ => tool.to_string(),
    }
}

fn summary_of(a: &Action) -> String { format!("{} {}", a.tool, a.target.clone().unwrap_or_default()).trim().into() }

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n { return s.into(); }
    let cut = (0..=n).rev().find(|i| s.is_char_boundary(*i)).unwrap();
    format!("{}…", &s[..cut])
}
