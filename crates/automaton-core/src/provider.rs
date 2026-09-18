//! Provider 계약 + 스크립티드/실제 구현. M1은 테스트용 스크립티드가 기본.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Message { pub role: String, pub content: String }

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall { pub name: String, pub args: Value }

#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem { Delta(String), ToolCall(ToolCall) }

pub struct CompletionRequest {
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<(String, String, Value)>, // (name, description, schema)
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{0}")] Message(String),
    #[error("provider: {0}")] Provider(String),
}

/// 프로바이더 계약 — 완성 스트림을 반환 (M1 단순화: Vec; 청크 스트리밍은 Chunk 4 데몬에서 개량)
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError>;
}

/// OpenAI 호환 HTTP 프로바이더 (키 재사용 — §3 Providers). M1 검증은 수동(env 키 필요), CI는 Scripted 사용.
pub struct OpenAiCompat { pub base_url: String, pub api_key: String, pub model: String, pub http: reqwest::Client }

impl OpenAiCompat {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("AUTOMATON_API_KEY").ok()?;
        let base_url = std::env::var("AUTOMATON_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into());
        let model = std::env::var("AUTOMATON_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
        Some(Self { base_url, api_key, model, http: reqwest::Client::new() })
    }
}

#[derive(serde::Deserialize)]
struct ChatResp { choices: Vec<Choice> }
#[derive(serde::Deserialize)]
struct Choice { message: RespMessage }
#[derive(serde::Deserialize)]
struct RespMessage { content: Option<String>, tool_calls: Option<Vec<RawToolCall>> }
#[derive(serde::Deserialize)]
struct RawToolCall { function: RawFunction }
#[derive(serde::Deserialize)]
struct RawFunction { name: String, arguments: String }

#[async_trait::async_trait]
impl Provider for OpenAiCompat {
    async fn complete(&self, req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError> {
        let tools: Vec<Value> = req.tools.iter().map(|(n, d, s)| serde_json::json!({
            "type": "function",
            "function": {"name": n, "description": d, "parameters": s}
        })).collect();
        let messages: Vec<Value> = std::iter::once(serde_json::json!({"role": "system", "content": req.system}))
            .chain(req.messages.iter().map(|m| serde_json::json!({"role": m.role, "content": m.content})))
            .collect();
        let body = serde_json::json!({"model": self.model, "messages": messages, "tools": tools});
        let resp = self.http.post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key).json(&body).send().await
            .map_err(|e| CoreError::Provider(e.to_string()))?;
        let chat: ChatResp = resp.json().await.map_err(|e| CoreError::Provider(e.to_string()))?;
        let m = chat.choices.into_iter().next().ok_or_else(|| CoreError::Provider("빈 응답".into()))?.message;
        let mut items = vec![];
        if let Some(t) = m.content { items.push(StreamItem::Delta(t)); }
        for c in m.tool_calls.unwrap_or_default() {
            let args: Value = serde_json::from_str(&c.function.arguments).unwrap_or(Value::Null);
            items.push(StreamItem::ToolCall(ToolCall { name: c.function.name, args }));
        }
        Ok(items)
    }
}
