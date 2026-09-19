//! automaton-memory — 단일 SQLite 스토어: 세션·요약·사실·결정 (§7 메모리 3계층)

use automaton_proto::Message;
use rusqlite::{Connection, Row};

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
}

/// FTS5 MATCH 이스케이프: 각 토큰을 `"..."*` 인용 접두 형태로 — 내부 `"`는 `""` doubling.
/// 원시 토큰 그대로면 점·괄호·예약어(AND 등)에서 하드 에러(실측).
fn fts_escape(q: &str) -> String {
    q.split_whitespace().map(|t| format!("\"{}\"*", t.replace('"', "\"\""))).collect::<Vec<_>>().join(" ")
}

pub mod skills;
pub use skills::*;
