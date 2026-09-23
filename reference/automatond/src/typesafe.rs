//! TypeSafe 클라이언트 — Jev System One 모델로 의미적 판단.
//! Apprentice 힌트의 토큰 중첩 대칭 신호를 보완하는 캘리브레이션된 확률 제공.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum TypeSafeError {
    #[error("http: {0}")] Http(String),
    #[error("api: {0}")] Api(String),
}

#[derive(Serialize)]
struct NoulQuestion {
    #[serde(rename = "type")]
    qtype: String,
    instructions: String,
    criteria: Option<NoulCriteria>,
}

#[derive(Serialize)]
struct NoulCriteria {
    #[serde(rename = "true")]
    yes: String,
    #[serde(rename = "false")]
    no: String,
}

#[derive(Serialize)]
struct EvaluateRequest {
    state: String,
    model: String,
    questions: HashMap<String, NoulQuestion>,
}

#[derive(Deserialize)]
struct EvaluateResponse {
    answers: HashMap<String, NoulAnswer>,
}

#[derive(Deserialize)]
struct NoulAnswer {
    noul: f64,
}

pub struct TypeSafeClient {
    http: reqwest::Client,
    api_key: String,
}

impl TypeSafeClient {
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("TYPESAFE_API_KEY").ok()?;
        Some(Self { http: reqwest::Client::new(), api_key: key })
    }

    /// "사용자가 이 도구 호출을 승인할 확률" — Noul 질문으로 캘리브레이션된 확률 반환
    pub async fn approval_probability(&self, tool: &str, target: &str, past_decisions: &[(String, String, String)]) -> Result<f64, TypeSafeError> {
        // 과거 결정을 state에 포함 — Jev가 맥락 파악
        let history: Vec<String> = past_decisions.iter()
            .map(|(t, tgt, d)| format!("[{}] {} → {}", t, tgt, d))
            .collect();

        let state = format!(
            "Tool call: {} {}\nTarget: {}\n\nPast user decisions:\n{}",
            tool, target, target,
            if history.is_empty() { "(no history)".to_string() } else { history.join("\n") }
        );

        let mut questions = HashMap::new();
        questions.insert("will_approve".to_string(), NoulQuestion {
            qtype: "noul".to_string(),
            instructions: "Based on the past decisions, will the user approve this tool call?".to_string(),
            criteria: Some(NoulCriteria {
                yes: "The user has approved similar tool calls with similar targets".to_string(),
                no: "The user has denied similar tool calls or this is a new/risky action".to_string(),
            }),
        });

        let req = EvaluateRequest { state, model: "jev-latest".to_string(), questions };
        let resp = self.http
            .post("https://api.typesafe.ai/v1/systemone")
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await
            .map_err(|e| TypeSafeError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(TypeSafeError::Api(format!("HTTP {} — {}", status, &body[..body.len().min(200)])));
        }

        let eval: EvaluateResponse = resp.json().await.map_err(|e| TypeSafeError::Api(e.to_string()))?;
        let answer = eval.answers.get("will_approve")
            .ok_or_else(|| TypeSafeError::Api("missing answer".into()))?;
        Ok(answer.noul)
    }
}
