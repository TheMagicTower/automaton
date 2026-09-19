//! automaton-memory — 단일 SQLite 스토어: 세션·요약·사실·결정 (§7 메모리 3계층)

use automaton_proto::Message;
use rusqlite::{params, Connection, Row};

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("sqlite: {0}")] Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")] Io(#[from] std::io::Error),
}

pub type Result<T, E = MemoryError> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub session: String,
    pub tool: String,
    pub target: String,
    pub verdict: String,
    pub decision: String,
}

pub struct MemoryStore { conn: parking_lot::Mutex<Connection> } // parking_lot — 즉시 unwrap 락은 rs-parking-lot 룰 적용 + rusqlite Connection !Sync의 Sync화 (계획 수정 8d94474)

impl MemoryStore {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?; // 데몬의 store·Apprentice 이중 연결 쓰기 충돌 대비 (리뷰 자문)
        conn.execute_batch("
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS messages(id INTEGER PRIMARY KEY, session TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, at TEXT DEFAULT (datetime('now')));
            CREATE TABLE IF NOT EXISTS summaries(session TEXT PRIMARY KEY, summary TEXT NOT NULL, at TEXT DEFAULT (datetime('now')));
            CREATE VIRTUAL TABLE IF NOT EXISTS facts_fts USING fts5(content);
            CREATE VIRTUAL TABLE IF NOT EXISTS decisions_fts USING fts5(session UNINDEXED, tool UNINDEXED, target, verdict UNINDEXED, decision UNINDEXED);
        ")?;
        Ok(MemoryStore { conn: parking_lot::Mutex::new(conn) })
    }

    pub fn append_message(&self, session: &str, role: &str, content: &str) -> Result<()> {
        self.conn.lock().execute("INSERT INTO messages(session, role, content) VALUES (?1, ?2, ?3)", (session, role, content))?;
        Ok(())
    }

    pub fn messages(&self, session: &str) -> Result<Vec<Message>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT role, content FROM messages WHERE session = ?1 ORDER BY id")?;
        let rows = stmt.query_map([session], |r: &Row| Ok(Message { role: r.get(0)?, content: r.get(1)? }))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn save_summary(&self, session: &str, summary: &str) -> Result<()> {
        self.conn.lock().execute("INSERT INTO summaries(session, summary) VALUES (?1, ?2) ON CONFLICT(session) DO UPDATE SET summary = ?2, at = datetime('now')", (session, summary))?;
        Ok(())
    }

    pub fn search_summaries(&self, query: &str) -> Result<Vec<String>> {
        // M1 위임: 스펙 §7 작업 계층은 'FTS5 + 벡터' 명시 — M1은 LIKE 근사, FTS/벡터는 후속 계획(§7 위임 사항)
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT summary FROM summaries WHERE summary LIKE ?1")?;
        let pat = format!("%{query}%");
        let rows = stmt.query_map([&pat], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 최근 요약 limit개 (저장 역순) — 세션 시작 시 '이전 세션 요약' 주입용. exclude_session(자기 자신) 제외.
    pub fn recent_summaries(&self, limit: usize, exclude_session: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT summary FROM summaries WHERE session != ?1 ORDER BY at DESC, rowid DESC LIMIT ?2")?;
        let rows = stmt.query_map(params![exclude_session, limit as i64], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn add_fact(&self, content: &str) -> Result<()> {
        self.conn.lock().execute("INSERT INTO facts_fts(content) VALUES (?1)", (content,))?;
        Ok(())
    }

    pub fn search_facts(&self, query: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT content FROM facts_fts WHERE facts_fts MATCH ?1")?;
        let rows = stmt.query_map([fts_escape(query)], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 관련도 검색 (§7): 키워드 추출(한글 조사 제거) → 동의어 확장 → FTS OR 수집 → 관련도 랭킹.
    /// 원본 키워드 hit는 2점, 동의어 hit는 1점 — 동점은 최근(rowid 큰) 것 우선.
    /// 결과가 없으면 최근 facts limit개로 폴백 (빈 결과보다 최근 맥락이 유용).
    pub fn search_facts_related(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let keywords = extract_keywords(query);
        if keywords.is_empty() { return self.recent_facts(limit); }
        let terms = expand_synonyms(&keywords); // (용어, 원본 여부)
        let fts_query = terms.iter()
            .map(|(t, _)| format!("\"{}\"*", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let mut scored = {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare("SELECT rowid, content FROM facts_fts WHERE facts_fts MATCH ?1")?;
            let rows = stmt.query_map([&fts_query], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            let mut out = Vec::new();
            for row in rows {
                let (rowid, content) = row?;
                let lower = content.to_lowercase();
                let score: u32 = terms.iter().map(|(t, primary)| lower.contains(t.as_str()) as u32 * if *primary { 2 } else { 1 }).sum();
                out.push((rowid, score, content));
            }
            out
        }; // 락 가드 해제 후 폴백 재쿼리 가능
        if scored.is_empty() { return self.recent_facts(limit); }
        scored.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
        Ok(scored.into_iter().take(limit).map(|(_, _, c)| c).collect())
    }

    /// 최근 facts limit개 (rowid 역순 = 저장 역순) — search_facts_related 폴백
    pub fn recent_facts(&self, limit: usize) -> Result<Vec<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT content FROM facts_fts ORDER BY rowid DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 전체 facts 페이지 조회 (rowid 순 = 저장 순) + 전체 개수 — MemoryBrowse용
    pub fn facts_page(&self, offset: usize, limit: usize) -> Result<(Vec<String>, usize)> {
        let conn = self.conn.lock();
        let total: usize = conn.query_row("SELECT COUNT(*) FROM facts_fts", [], |r| r.get(0))?;
        let mut stmt = conn.prepare("SELECT content FROM facts_fts ORDER BY rowid LIMIT ?1 OFFSET ?2")?;
        let rows = stmt.query_map([limit as i64, offset as i64], |r| r.get::<_, String>(0))?;
        Ok((rows.collect::<std::result::Result<Vec<_>, _>>()?, total))
    }

    /// content와 정확히 일치하는 fact 삭제 — 삭제된 행 수 반환 (0 = 해당 없음) — MemoryDelete용
    pub fn delete_fact(&self, content: &str) -> Result<usize> {
        Ok(self.conn.lock().execute("DELETE FROM facts_fts WHERE content = ?1", (content,))?)
    }

    pub fn record_decision(&self, session: &str, tool: &str, target: &str, verdict: &str, decision: &str) -> Result<()> {
        self.conn.lock().execute("INSERT INTO decisions_fts(session, tool, target, verdict, decision) VALUES (?1, ?2, ?3, ?4, ?5)", (session, tool, target, verdict, decision))?;
        Ok(())
    }

    pub fn search_decisions(&self, query: &str) -> Result<Vec<Decision>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT session, tool, target, verdict, decision FROM decisions_fts WHERE decisions_fts MATCH ?1")?;
        let rows = stmt.query_map([fts_escape(query)], |r: &Row| Ok(Decision {
            session: r.get(0)?, tool: r.get(1)?, target: r.get(2)?, verdict: r.get(3)?, decision: r.get(4)?,
        }))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 사전 구성된 FTS 쿼리 그대로 MATCH — 호출자(Apprentice)가 인용 접두·OR 형태를 직접 구성할 때 사용.
    /// 일반 텍스트 검색에는 search_decisions(fts_escape 자동 적용)를 쓸 것.
    pub fn search_decisions_fts(&self, raw_fts_query: &str) -> Result<Vec<Decision>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT session, tool, target, verdict, decision FROM decisions_fts WHERE decisions_fts MATCH ?1")?;
        let rows = stmt.query_map([raw_fts_query], |r: &Row| Ok(Decision {
            session: r.get(0)?, tool: r.get(1)?, target: r.get(2)?, verdict: r.get(3)?, decision: r.get(4)?,
        }))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 툴별 결정 이력 전체 조회 — tool 칼럼은 UNINDEXED라 FTS MATCH로 못 얻음.
    pub fn decisions_by_tool(&self, tool: &str) -> Result<Vec<Decision>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT session, tool, target, verdict, decision FROM decisions_fts WHERE tool = ?1")?;
        let rows = stmt.query_map([tool], |r: &Row| Ok(Decision {
            session: r.get(0)?, tool: r.get(1)?, target: r.get(2)?, verdict: r.get(3)?, decision: r.get(4)?,
        }))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// (fact 수, 세션 수, 결정 수) — MemoryStats용
    pub fn stats(&self) -> Result<(usize, usize, usize)> {
        let conn = self.conn.lock();
        let count = |sql: &str| -> Result<usize> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
        Ok((
            count("SELECT COUNT(*) FROM facts_fts")?,
            count("SELECT COUNT(DISTINCT session) FROM messages")?,
            count("SELECT COUNT(*) FROM decisions_fts")?,
        ))
    }
}

/// FTS5 MATCH 이스케이프: 각 토큰을 `"..."*` 인용 접두 형태로 — 내부 `"`는 `""` doubling.
/// 원시 토큰 그대로면 점·괄호·예약어(AND 등)에서 하드 에러(실측).
fn fts_escape(q: &str) -> String {
    q.split_whitespace().map(|t| format!("\"{}\"*", t.replace('"', "\"\""))).collect::<Vec<_>>().join(" ")
}

/// 한글 조사 접미사 — 긴 것부터 시도 ("에서는"을 "는"으로 자르면 어간에 조사 잔여물이 남음).
/// 요구 목록(은/는/이/가/을/를/의/에/에서/으로)에 빈출 조사 보강.
const PARTICLES: &[&str] = &[
    "에서는", "에게는", "으로는", "이라는", "이라고", "에서", "에게", "으로", "한테", "부터", "까지", "처럼", "라고",
    "의", "은", "는", "이", "가", "을", "를", "에", "와", "과", "도", "만", "로", "랑", "께",
];

/// 검색 확장 동의어 그룹 — 그룹 내 단어는 서로 동치로 확장 (이름/성함, 좋아함/선호, 프로젝트/작업, 파일/문서, 설정/구성)
pub const SYNONYM_GROUPS: &[&[&str]] = &[
    &["이름", "성함", "성명"],
    &["좋아함", "좋아하는", "선호", "선호하는", "취향"],
    &["프로젝트", "작업", "업무"],
    &["파일", "문서", "폴더", "디렉터리"],
    &["설정", "구성", "환경설정"],
];

/// 검색 키워드 추출: 비영숫자 분리 → 한글 조사 제거 → 소문자화 → 중복·1글자 제거
pub fn extract_keywords(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tok in query.split(|c: char| !c.is_alphanumeric()) {
        let k = strip_particle(tok).to_lowercase();
        if k.chars().count() >= 2 && !out.contains(&k) { out.push(k); }
    }
    out
}

/// 토큰 끝에서 가장 긴 조사 하나를 제거 (어간이 빈 문자열이면 제거하지 않음)
fn strip_particle(tok: &str) -> &str {
    for p in PARTICLES {
        if let Some(stem) = tok.strip_suffix(p) {
            if !stem.is_empty() { return stem; }
        }
    }
    tok
}

/// 키워드 → (용어, 원본 여부) 확장 — 원본 키워드 우선, 동의어는 중복 제거해 부착
fn expand_synonyms(keywords: &[String]) -> Vec<(String, bool)> {
    let mut terms: Vec<(String, bool)> = keywords.iter().map(|k| (k.clone(), true)).collect();
    for k in keywords {
        for group in SYNONYM_GROUPS {
            if group.contains(&k.as_str()) {
                for syn in group.iter() {
                    if *syn != k.as_str() && !terms.iter().any(|(t, _)| t.as_str() == *syn) {
                        terms.push((syn.to_string(), false));
                    }
                }
            }
        }
    }
    terms
}

pub mod skills;
pub use skills::*;
