//! Provider 계약 + 스크립티드/실제 구현. M1은 테스트용 스크립티드가 기본.

use futures_util::{Stream, StreamExt};
use serde_json::Value;
use std::collections::VecDeque;
use std::fmt;
use std::pin::Pin;

/// Message는 automaton-proto로 이전 — core 재수출로 기존 경로(`automaton_core::Message`) 호환 유지.
pub use automaton_proto::Message;
use automaton_proto::TokenUsage;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub args: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    Delta(String),
    ToolCall(ToolCall),
    /// 완성 1회의 API 사용량 — track_usage 요청에만 발생 (감사 로그行)
    Usage(TokenUsage),
}

/// 프로바이더 청크 스트림 — 도착 즉시 소비 (Send 박스)
pub type ProviderStream = Pin<Box<dyn Stream<Item = Result<StreamItem, CoreError>> + Send>>;

pub struct CompletionRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<(String, String, Value)>, // (name, description, schema)
    /// true면 usage 필드를 요청·파싱해 StreamItem::Usage로 전달
    pub track_usage: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{0}")]
    Message(String),
    #[error("provider: {0}")]
    Provider(String),
}

/// 프로바이더 계약 — complete()는 완성 전체(Vec), complete_stream()은 청크 즉시 스트리밍.
/// 기본 complete_stream()은 complete()를 버퍼링 재생으로 감쌈 — 스크립티드 더블은 complete()만 구현하면 된다.
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError>;

    async fn complete_stream(&self, req: CompletionRequest) -> Result<ProviderStream, CoreError> {
        let items = self.complete(req).await?;
        Ok(futures_util::stream::iter(items.into_iter().map(Ok)).boxed())
    }
}

/// OpenAI 호환 HTTP 프로바이더 (키 재사용 — §3 Providers). M1 검증은 수동(env 키 필요), CI는 Scripted 사용.
pub struct OpenAiCompat {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub http: reqwest::Client,
}

impl OpenAiCompat {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("AUTOMATON_API_KEY").ok()?;
        let base_url = std::env::var("AUTOMATON_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".into());
        let model = std::env::var("AUTOMATON_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
        Some(Self {
            base_url,
            api_key,
            model,
            http: reqwest::Client::new(),
        })
    }

    /// 공통 요청 본문 — stream=true면 SSE 청크, track_usage면 usage 포함 요청
    fn request_body(&self, req: &CompletionRequest, stream: bool) -> Value {
        let tools: Vec<Value> = req
            .tools
            .iter()
            .map(|(n, d, s)| {
                serde_json::json!({
                    "type": "function",
                    "function": {"name": n, "description": d, "parameters": s}
                })
            })
            .collect();
        let messages: Vec<Value> =
            std::iter::once(serde_json::json!({"role": "system", "content": req.system}))
                .chain(
                    req.messages
                        .iter()
                        .map(|m| serde_json::json!({"role": m.role, "content": m.content})),
                )
                .collect();
        let mut body = serde_json::json!({"model": self.model, "messages": messages, "tools": tools, "stream": stream});
        if stream && req.track_usage {
            body["stream_options"] = serde_json::json!({"include_usage": true});
        }
        body
    }

    async fn send(&self, body: &Value) -> Result<reqwest::Response, CoreError> {
        self.http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(body)
            .send()
            .await
            .map_err(|e| CoreError::Provider(format!("네트워크 오류: {e}")))
    }

    /// 비성공 응답 → 오류 본문 detail 추출 (잔액 부족·인증 실패 등)
    async fn error_for_status(resp: reqwest::Response) -> CoreError {
        let status = resp.status();
        let raw = resp.text().await.unwrap_or_default();
        let detail = serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|v| {
                v.get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| raw.chars().take(200).collect());
        CoreError::Provider(format!("API 오류 (HTTP {}): {}", status.as_u16(), detail))
    }
}

#[derive(serde::Deserialize)]
struct ChatResp {
    choices: Vec<Choice>,
    usage: Option<TokenUsage>,
}
#[derive(serde::Deserialize)]
struct Choice {
    message: RespMessage,
}
#[derive(serde::Deserialize)]
struct RespMessage {
    content: Option<String>,
    tool_calls: Option<Vec<RawToolCall>>,
}
#[derive(serde::Deserialize)]
struct RawToolCall {
    function: RawFunction,
}
#[derive(serde::Deserialize)]
struct RawFunction {
    name: String,
    arguments: String,
}

#[derive(serde::Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>, // usage-only 마지막 청크는 빈 choices
    usage: Option<TokenUsage>,
}
#[derive(serde::Deserialize)]
struct StreamChoice {
    delta: Option<StreamDelta>,
}
#[derive(serde::Deserialize)]
struct StreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<StreamToolCall>>,
}
#[derive(serde::Deserialize)]
struct StreamToolCall {
    index: usize,
    function: Option<StreamFunction>,
}
#[derive(serde::Deserialize)]
struct StreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Default, Clone)]
struct ToolAccum {
    name: String,
    args: String,
}

/// SSE 파서 상태 — 라인 버퍼(청크 경계 절단 결합) + 툴콜 파편 누적 + 발행 대기 큐
struct SseState<S> {
    bytes: S,
    buf: Vec<u8>,
    tools: Vec<ToolAccum>,
    pending: VecDeque<StreamItem>,
    usage: Option<TokenUsage>,
    finished: bool,
}

impl<S> SseState<S> {
    /// 완성된 SSE 라인 1개 반영 — data: 페이로드만, 그 외(공백·주석·이벤트 필드)는 무시
    fn feed(&mut self, line: &str) {
        let Some(payload) = line.strip_prefix("data:") else {
            return;
        };
        let payload = payload.trim();
        if payload == "[DONE]" {
            self.finished = true;
            self.finish();
            return;
        }
        if payload.is_empty() {
            return;
        }
        let Ok(chunk) = serde_json::from_str::<StreamChunk>(payload) else {
            return;
        }; // 알 수 없는 청크 형식 무시
        if let Some(u) = chunk.usage {
            self.usage = Some(u);
        }
        for choice in chunk.choices {
            let Some(delta) = choice.delta else { continue };
            if let Some(c) = delta.content
                && !c.is_empty()
            {
                self.pending.push_back(StreamItem::Delta(c));
            }
            for tc in delta.tool_calls.unwrap_or_default() {
                if self.tools.len() <= tc.index {
                    self.tools.resize(tc.index + 1, ToolAccum::default());
                }
                if let Some(f) = tc.function {
                    if let Some(n) = f.name
                        && !n.is_empty()
                    {
                        self.tools[tc.index].name.push_str(&n);
                    }
                    if let Some(a) = f.arguments {
                        self.tools[tc.index].args.push_str(&a);
                    }
                }
            }
        }
    }

    /// [DONE]/EOF 확정 — 누적 툴콜 파편을 아이템으로 확정, usage는 마지막에 1회
    fn finish(&mut self) {
        for t in std::mem::take(&mut self.tools) {
            let args: Value = serde_json::from_str(&t.args).unwrap_or(Value::Null);
            self.pending
                .push_back(StreamItem::ToolCall(ToolCall { name: t.name, args }));
        }
        if let Some(u) = self.usage.take() {
            self.pending.push_back(StreamItem::Usage(u));
        }
    }
}

fn sse_items<S, B, E>(bytes: S) -> ProviderStream
where
    S: Stream<Item = Result<B, E>> + Send + Unpin + 'static,
    B: AsRef<[u8]>,
    E: fmt::Display,
{
    let state = SseState {
        bytes,
        buf: Vec::new(),
        tools: Vec::new(),
        pending: VecDeque::new(),
        usage: None,
        finished: false,
    };
    futures_util::stream::unfold(state, |mut st| async move {
        loop {
            if let Some(item) = st.pending.pop_front() {
                return Some((Ok(item), st));
            }
            if st.finished {
                return None;
            }
            match st.buf.iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    let line: Vec<u8> = st.buf.drain(..=nl).collect();
                    let line = String::from_utf8_lossy(&line); // 완성 라인만 변환 — 경계 절단 멀티바이트 안전
                    st.feed(line.trim_end_matches(['\r', '\n']));
                }
                None => match st.bytes.next().await {
                    Some(Ok(b)) => st.buf.extend_from_slice(b.as_ref()),
                    Some(Err(e)) => {
                        st.finished = true;
                        return Some((
                            Err(CoreError::Provider(format!("스트림 읽기 실패: {e}"))),
                            st,
                        ));
                    }
                    None => {
                        st.finished = true;
                        st.finish();
                    } // [DONE] 없이 종결된 스트림 — 누적분 확정
                },
            }
        }
    })
    .boxed()
}

#[async_trait::async_trait]
impl Provider for OpenAiCompat {
    async fn complete(&self, req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError> {
        let body = self.request_body(&req, false);
        let resp = self.send(&body).await?;
        // HTTP 상태 확인 — API 오류 본문을 그대로 노출 (잔액 부족·인증 실패 등)
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        let raw = resp
            .text()
            .await
            .map_err(|e| CoreError::Provider(format!("응답 읽기 실패: {e}")))?;
        let chat: ChatResp = serde_json::from_str(&raw).map_err(|e| {
            CoreError::Provider(format!(
                "응답 파싱 실패: {e} — 본문: {}",
                &raw.chars().take(200).collect::<String>()
            ))
        })?;
        let m = chat
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::Provider("빈 응답".into()))?
            .message;
        let mut items = vec![];
        if let Some(t) = m.content {
            items.push(StreamItem::Delta(t));
        }
        for c in m.tool_calls.unwrap_or_default() {
            let args: Value = serde_json::from_str(&c.function.arguments).unwrap_or(Value::Null);
            items.push(StreamItem::ToolCall(ToolCall {
                name: c.function.name,
                args,
            }));
        }
        if req.track_usage
            && let Some(u) = chat.usage
        {
            items.push(StreamItem::Usage(u));
        }
        Ok(items)
    }

    async fn complete_stream(&self, req: CompletionRequest) -> Result<ProviderStream, CoreError> {
        let body = self.request_body(&req, true);
        let resp = self.send(&body).await?;
        if !resp.status().is_success() {
            return Err(Self::error_for_status(resp).await);
        }
        Ok(sse_items(Box::pin(resp.bytes_stream())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(parts: Vec<Vec<u8>>) -> impl Stream<Item = Result<Vec<u8>, std::io::Error>> {
        futures_util::stream::iter(parts.into_iter().map(Ok))
    }

    async fn collect(st: ProviderStream) -> Vec<StreamItem> {
        st.map(|r| r.unwrap()).collect().await
    }

    #[tokio::test]
    async fn sse_content_deltas_stream_in_order() {
        let raw = "data: {\"choices\":[{\"delta\":{\"content\":\"안녕\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"하세요\"}}]}\n\ndata: [DONE]\n\n";
        let items = collect(sse_items(chunks(vec![raw.as_bytes().to_vec()]))).await;
        assert_eq!(
            items,
            vec![
                StreamItem::Delta("안녕".into()),
                StreamItem::Delta("하세요".into())
            ]
        );
    }

    #[tokio::test]
    async fn sse_line_split_across_chunks_is_reassembled() {
        let parts: Vec<Vec<u8>> = vec![
            b"data: {\"choices\":[{\"delta\":{\"con".to_vec(),
            "tent\":\"잘\"}}]}\n".as_bytes().to_vec(),
            b"\ndata: [DONE]\n".to_vec(),
        ];
        let items = collect(sse_items(chunks(parts))).await;
        assert_eq!(items, vec![StreamItem::Delta("잘".into())]);
    }

    #[tokio::test]
    async fn sse_multibyte_char_split_across_chunks_survives() {
        let raw = "data: {\"choices\":[{\"delta\":{\"content\":\"한\"}}]}\n\ndata: [DONE]\n";
        let bytes = raw.as_bytes();
        let cut = raw.find("한").unwrap() + 1; // 멀티바이트 문자 중간에서 절단
        let parts: Vec<Vec<u8>> = vec![bytes[..cut].to_vec(), bytes[cut..].to_vec()];
        let items = collect(sse_items(chunks(parts))).await;
        assert_eq!(items, vec![StreamItem::Delta("한".into())]);
    }

    #[tokio::test]
    async fn sse_tool_call_fragments_accumulate_then_flush() {
        // json! 매크로로 조립 — arguments 내부 따옴표 이스케이프를 손으로 쓰면 실측 오류(잘못 이스케이프된 청크 드롭)
        let chunk = |v: Value| format!("data: {v}\n\n");
        let raw = format!(
            "{}{}data: [DONE]\n\n",
            chunk(
                serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"fs.write","arguments":"{\"pa"}}]}}]})
            ),
            chunk(
                serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a.txt\"}"}}]}}]})
            ),
        );
        let items = collect(sse_items(chunks(vec![raw.as_bytes().to_vec()]))).await;
        assert_eq!(items.len(), 1);
        match &items[0] {
            StreamItem::ToolCall(c) => {
                assert_eq!(c.name, "fs.write");
                assert_eq!(c.args, serde_json::json!({"path": "a.txt"}));
            }
            other => panic!("툴콜 아님: {other:?}"),
        }
    }

    #[tokio::test]
    async fn sse_usage_only_final_chunk_emits_usage() {
        let raw = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n",
            "data: [DONE]\n\n",
        );
        let items = collect(sse_items(chunks(vec![raw.as_bytes().to_vec()]))).await;
        assert_eq!(
            items,
            vec![
                StreamItem::Delta("ok".into()),
                StreamItem::Usage(TokenUsage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15
                }),
            ]
        );
    }

    #[tokio::test]
    async fn sse_ignores_comments_and_non_data_lines() {
        let raw = ": keep-alive 주석\nevent: message\n\ndata: {\"choices\":[{\"delta\":{}}]}\n\ndata: [DONE]\n";
        let items = collect(sse_items(chunks(vec![raw.as_bytes().to_vec()]))).await;
        assert!(items.is_empty());
    }
}
