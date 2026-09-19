//! automaton-apprentice 1단계 (§6) — 결정 저널 + 유사 결정 kNN(FTS 근사) 힌트.
//! 출력은 참고용 어드바이저. Policy Engine을 우회하지 않는다(§6 안전장치 — 루프는 hint를 이벤트에만 싣는다).

use automaton_memory::MemoryStore;
use automaton_proto::{ActionInfo, Hint};

#[derive(Debug, thiserror::Error)]
pub enum ApprenticeError {
    #[error("memory: {0}")] Memory(#[from] automaton_memory::MemoryError),
}

pub type Result<T, E = ApprenticeError> = std::result::Result<T, E>;

pub struct Apprentice { store: MemoryStore }

impl Apprentice {
    pub fn open(db_path: &std::path::Path) -> Result<Self> {
        Ok(Apprentice { store: MemoryStore::open(db_path)? })
    }

    /// 모든 정책 결정 기록 — allow 포함 (§5 감사 데이터가 학습 데이터가 된다)
    pub fn note_decision(&self, session: &str, action: &ActionInfo, verdict: &str, decision: &str) -> Result<()> {
        Ok(self.store.record_decision(session, &action.tool, &action.target, verdict, decision)?)
    }

    /// 유사 과거 승인 검색 → 승인 배너 힌트. 승인만 집계(거절은 힌트 근거 아님).
    /// OR 접두 쿼리로 후보 수집 → Rust측 토큰 중첩(≥2, 단일 토큰 쿼리는 1) 랭킹.
    /// AND 결합은 파일명만 달라도 0히트로 힌트가 사실상 발화하지 않음(실측).
    pub fn hint_for(&self, action: &ActionInfo) -> Result<Option<Hint>> {
        let tokens: Vec<String> = action.target.split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty()).map(|t| t.to_lowercase()).collect();
        if tokens.is_empty() { return Ok(None); }
        let query = tokens.iter().map(|t| format!("\"{t}\"*")).collect::<Vec<_>>().join(" OR ");
        let hits = self.store.search_decisions_fts(&query)?;
        let min_overlap = if tokens.len() == 1 { 1 } else { 2 };
        let mut approvals = 0u32;
        for d in hits {
            if d.decision != "approve" || d.tool != action.tool { continue; }
            let past: std::collections::HashSet<String> = d.target.split(|c: char| !c.is_alphanumeric())
                .filter(|t| !t.is_empty()).map(|t| t.to_lowercase()).collect();
            let overlap = tokens.iter().filter(|t| past.contains(*t)).count();
            if overlap >= min_overlap { approvals += 1; }
        }
        if approvals == 0 { return Ok(None); }
        Ok(Some(Hint { text: format!("지난번 유사 상황에서 승인({approvals}회)"), similar_count: approvals }))
    }
}

/// 2단계(§6) — 임베딩 위 소형 분류기 대신, 결정 저널의 툴별 승인 비율로
/// P(approve)를 예측해 배너 기본값을 사전 선택하는 어드바이저.
/// 출력은 힌트일 뿐 — Policy Engine 우회 없음.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreferenceScore {
    pub probability: f64,
    pub confidence: f64,
    pub sample_count: u32,
    pub suggested_default: bool,
}

pub struct PreferenceScorer<'a> { apprentice: &'a Apprentice }

impl PreferenceScorer<'_> {
    /// 툴별 승인/거절 이력 기반 P(approve) 점수.
    /// 신뢰도는 표본 수 기반: n>=3이면 1.0(높음), n<3이면 n/3(낮음).
    pub fn score(&self, tool: &str) -> Result<PreferenceScore> {
        let mut approve = 0u32;
        let mut deny = 0u32;
        for d in self.apprentice.store.decisions_by_tool(tool)? {
            match d.decision.as_str() {
                "approve" => approve += 1,
                "deny" => deny += 1,
                _ => {}
            }
        }
        let n = approve + deny;
        let probability = if n == 0 { 0.5 } else { approve as f64 / n as f64 };
        let confidence = if n >= 3 { 1.0 } else { n as f64 / 3.0 };
        Ok(PreferenceScore {
            probability,
            confidence,
            sample_count: n,
            suggested_default: probability > 0.8 && n >= 3 && confidence > 0.7,
        })
    }
}

impl Apprentice {
    /// 선호 스코어러를 통한 P(approve) 조회 — 배너 기본값 사전 선택용 어드바이저.
    pub fn preference_score(&self, tool: &str) -> Result<PreferenceScore> {
        PreferenceScorer { apprentice: self }.score(tool)
    }
}
