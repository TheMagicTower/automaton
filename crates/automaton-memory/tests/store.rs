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

#[test]
fn extract_keywords_strips_korean_particles() {
    let kw = automaton_memory::extract_keywords("caspar의 이름이 뭐야? 집에서는 뭘 해");
    assert!(kw.contains(&"caspar".to_string()), "조사 '의' 제거 실패: {kw:?}");
    assert!(kw.contains(&"이름".to_string()), "조사 '이' 제거 실패: {kw:?}");
    assert!(!kw.contains(&"집에서는".to_string()), "복합 조사 '에서는'이 통째로 남음: {kw:?}");
    assert!(!kw.iter().any(|k| k.chars().count() < 2), "1글자 토큰이 남음: {kw:?}");
}

#[test]
fn related_search_matches_despite_particles() {
    let s = MemoryStore::open(&tmp("particles")).unwrap();
    s.add_fact("사용자 이름은 카스파르다").unwrap();
    s.add_fact("선호 에디터는 vim이다").unwrap();
    let hits = s.search_facts_related("이름이 뭐야?", 5).unwrap();
    assert!(hits.iter().any(|f| f.contains("카스파르")), "조사 붙은 질의가 이름 fact에 걸려야 함: {hits:?}");
    assert!(!hits.iter().any(|f| f.contains("vim")), "무관 fact는 뒤로/제외: {hits:?}");
}

#[test]
fn related_search_expands_synonyms() {
    let s = MemoryStore::open(&tmp("synonyms")).unwrap();
    s.add_fact("사용자의 성함은 caspar다").unwrap(); // '성함' ↔ '이름' 동의어 그룹
    let hits = s.search_facts_related("이름", 5).unwrap();
    assert!(hits.iter().any(|f| f.contains("caspar")), "동의어 확장으로 성함 fact 검색되어야 함: {hits:?}");
}

#[test]
fn related_search_ranks_multi_keyword_hits_first() {
    let s = MemoryStore::open(&tmp("ranking")).unwrap();
    s.add_fact("프로젝트 이야기만 있는 기록").unwrap();   // 1 hit
    s.add_fact("새 프로젝트 설정 완료").unwrap();        // 2 hits — 위에 있어도 아래로 내려가야 함
    let hits = s.search_facts_related("프로젝트 설정", 5).unwrap();
    assert_eq!(hits.first().map(String::as_str), Some("새 프로젝트 설정 완료"), "다중 키워드 hit가 먼저: {hits:?}");
}

#[test]
fn related_search_falls_back_to_recent_facts() {
    let s = MemoryStore::open(&tmp("fallback")).unwrap();
    for i in 0..5 { s.add_fact(&format!("기록-{i}")).unwrap(); }
    let hits = s.search_facts_related("zzz매칭없음", 3).unwrap();
    assert_eq!(hits.len(), 3, "무결과 시 최근 3개 폴백: {hits:?}");
    assert_eq!(hits[0], "기록-4", "최근(rowid 역순) 우선: {hits:?}");
}

#[test]
fn facts_page_delete_stats_roundtrip() {
    let s = MemoryStore::open(&tmp("browser")).unwrap();
    s.add_fact("fact-a").unwrap();
    s.add_fact("fact-b").unwrap();
    let (page, total) = s.facts_page(0, 1).unwrap();
    assert_eq!(page, vec!["fact-a".to_string()]);
    assert_eq!(total, 2);
    let (page2, _) = s.facts_page(1, 1).unwrap();
    assert_eq!(page2, vec!["fact-b".to_string()]);

    assert_eq!(s.delete_fact("fact-a").unwrap(), 1);
    assert_eq!(s.delete_fact("없는 fact").unwrap(), 0);
    let (rest, total) = s.facts_page(0, 10).unwrap();
    assert_eq!(rest, vec!["fact-b".to_string()]);
    assert_eq!(total, 1);

    s.append_message("s1", "user", "안녕").unwrap();
    s.record_decision("s1", "fs.delete", "x", "ask", "approve").unwrap();
    s.save_summary("s1", "요약1").unwrap();
    s.save_summary("s2", "요약2").unwrap();
    assert_eq!(s.stats().unwrap(), (1, 1, 1)); // fact / 세션 / 결정
    let recent = s.recent_summaries(3, "s2").unwrap();
    assert_eq!(recent, vec!["요약1".to_string()], "자기 세션 요약은 제외되어야 함");
}
