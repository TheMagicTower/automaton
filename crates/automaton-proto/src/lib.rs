//! automaton-proto — 셸↔코어 JSON-RPC 공개 계약 (텍스트 프로토콜, §4·§12)

use serde::{Deserialize, Serialize};

/// 대화 메시지 — 코어·메모리가 공유하는 기록 단위 (core에서 proto로 이전)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message { pub role: String, pub content: String }

/// 세션 모드 (§5)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode { Code, Mac, Chat }

/// 셸 → 코어 요청
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    SessionCreate { id: String },
    MessageSend { session: String, text: String },
    ApprovalRespond { session: String, approval: String, decision: Decision, always: bool },
    ModeSwitch { session: String, to: Mode },
    HistoryGet { session: String, limit: usize },
    /// 현재 턴 중단 — 장시간 셸 명령·응답 생성 취소
    Interrupt { session: String },
    SessionList,
    /// 메모리 브라우저 (§7) — 전체 facts 페이지네이션 조회
    MemoryBrowse { offset: usize, limit: usize },
    /// 메모리 브라우저 — content와 정확히 일치하는 fact 삭제
    MemoryDelete { content: String },
    /// 메모리 브라우저 — 총 fact 수·세션 수·결정 수
    MemoryStats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision { Approve, Deny }

/// 승인 대상 동작 요약 (배너 1줄 표시용)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionInfo {
    pub tool: String,
    pub target: String,
    pub risk: String,
}

/// API 토큰 사용량 — 프로바이더 응답의 usage 필드 (감사 로그 기록용)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// Apprentice 1단계 힌트 (§6)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hint {
    pub text: String,
    pub similar_count: u32,
}

/// 코어 → 셸 이벤트 (스트리밍)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    StreamDelta { session: String, delta: String },
    ToolStarted { session: String, tool: String, summary: String },
    ToolResult { session: String, tool: String, ok: bool, summary: String },
    ApprovalRequested { session: String, approval: String, action: ActionInfo, hint: Option<Hint> },
    /// Apprentice 3단계 답변 초안 (§6) — 승인 배너 직후 발행, 칩은 어드바이저일 뿐 승인 아님
    DraftSuggestions { session: String, suggestions: Vec<String> },
    /// 완성 1회당 API 사용량 리포트 — writer_loop가 감사 로그에 기록
    Usage { session: String, usage: TokenUsage },
    ModeChanged { session: String, mode: Mode },
    /// 메모리 브라우저 응답 — facts: 요청한 페이지, total: 전체 fact 수 (페이지네이션용)
    MemoryData { facts: Vec<String>, total: usize },
    /// 메모리 통계 응답 — fact·세션·결정 총계
    MemoryStats { facts: usize, sessions: usize, decisions: usize },
    Error { session: Option<String>, message: String },
}
