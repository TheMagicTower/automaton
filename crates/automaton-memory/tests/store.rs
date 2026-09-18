use automaton_memory::MemoryStore;

fn tmp(name: &str) -> std::path::PathBuf {
    // 테스트별 고유 서브디렉터리 — pid 공유 경로는 cargo test 병렬 실행에서 경합(실측: database is locked/SIGBUS)
    let dir = std::env::temp_dir().join(format!("automaton-memory-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("memory.db")
}

#[test]
fn opens_and_creates_schema_with_fts5() {
    let s = MemoryStore::open(&tmp("schema")).unwrap();
    // FTS5 가상 테이블이 실제로 동작하는지 확인 (bundled 빌드에 FTS5 없으면 여기서 실패)
    s.add_fact("caspar는 한국어를 쓴다").unwrap();
    assert_eq!(s.search_facts("한국어").unwrap().len(), 1);
}

#[test]
fn messages_roundtrip_per_session() {
    let s = MemoryStore::open(&tmp("messages")).unwrap();
    s.append_message("s1", "user", "안녕").unwrap();
    s.append_message("s1", "assistant", "안녕하세요").unwrap();
    s.append_message("s2", "user", "다른 세션").unwrap();
    let m = s.messages("s1").unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].content, "안녕");
}

#[test]
fn summary_upserts_and_searches() {
    let s = MemoryStore::open(&tmp("summaries")).unwrap();
    s.save_summary("s1", "다운로드 폴더 정리 작업").unwrap();
    s.save_summary("s1", "정리 작업 (개정)").unwrap(); // 같은 세션 upsert
    let hits = s.search_summaries("정리").unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].contains("개정"));
}

#[test]
fn decisions_record_and_fts_search() {
    let s = MemoryStore::open(&tmp("decisions")).unwrap();
    for i in 0..3 {
        s.record_decision("s1", "fs.delete", &format!("~/Downloads/old-{i}.zip"), "ask", "approve").unwrap();
    }
    s.record_decision("s2", "fs.delete", "~/Documents/plan.md", "ask", "deny").unwrap();
    let hits = s.search_decisions("Downloads").unwrap();
    assert_eq!(hits.len(), 3);
    assert!(hits.iter().all(|d| d.decision == "approve"));
}
