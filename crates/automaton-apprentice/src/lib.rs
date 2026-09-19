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
