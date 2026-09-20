//! 메모리 브라우저 RPC + 세션 요약기 E2E — 요청 왕복·삭제 후 재검색·유휴/새세션 요약·프롬프트 주입 검증.

use automaton_core::{CompletionRequest, CoreError, Provider, StreamItem};
use automaton_memory::MemoryStore;
use automatond::daemon::{Daemon, Paths};
use automaton_proto::Event;
use parking_lot::Mutex; // 즉시 lock — rs-parking-lot 룰
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn tmp_root(name: &str) -> PathBuf {
    // 테스트별 고유 루트 — pid 공유 시 같은 memory.db를 동시 open해 'database is locked' (실측, e2e.rs 참조)
    let dir = std::env::temp_dir().join(format!("automaton-memrpc-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

async fn read_events(reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>, until: &str) -> Vec<Event> {
    let mut evs = vec![];
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await.unwrap() == 0 { break; }
        let ev: Event = serde_json::from_str(line.trim()).unwrap();
        let done = format!("{ev:?}").contains(until);
        evs.push(ev);
        if done { break; }
    }
    evs
}

/// 시스템 프롬프트를 기록하는 프로바이더 — 요약 요청은 전용 프롬프트 마커로 구분해 짧은 요약 반환
struct Capturing { systems: Arc<Mutex<Vec<String>>> }
#[async_trait::async_trait]
impl Provider for Capturing {
    async fn complete(&self, req: CompletionRequest) -> Result<Vec<StreamItem>, CoreError> {
        self.systems.lock().push(req.system.clone());
        if req.system.contains("요약 도우미") {
            Ok(vec![StreamItem::Delta("다운로드 폴더 정리를 마쳤고 vim 설정을 바꿨다".into())])
        } else {
            Ok(vec![StreamItem::Delta("답변 완료".into())])
        }
    }
}

#[tokio::test]
async fn memory_browse_stats_delete_roundtrip() {
    let root = tmp_root("browse");
    // 데몬 기동 전 시드 — 데몬은 같은 DB 파일을 다시 연다 (WAL + busy_timeout으로 다중 연결 안전)
    {
        let seed = MemoryStore::open(&root.join("data/memory.db")).unwrap();
        seed.add_fact("caspar는 한국어를 쓴다").unwrap();
        seed.add_fact("선호 에디터는 vim").unwrap();
        seed.append_message("s0", "user", "이전 세션 대화").unwrap();
        seed.record_decision("s0", "fs.delete", "x.zip", "ask", "approve").unwrap();
        seed.save_summary("s0", "다운로드 폴더 정리를 마쳤고 vim 설정을 바꿨다").unwrap();
    }
    let systems = Arc::new(Mutex::new(Vec::new()));
    let provider = Capturing { systems: systems.clone() };
    let paths = Paths { data_dir: root.join("data"), config_dir: root.join("config") };
    let d = Arc::new(Daemon::new(Box::new(provider), paths));
    tokio::spawn(d.clone().serve(root.join("d.sock")));
    let mut stream = None;
    for _ in 0..20 {
        if let Ok(s) = tokio::net::UnixStream::connect(root.join("d.sock")).await { stream = Some(s); break; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let stream = stream.expect("데몬 소켓 연결 실패");
    let (rd, wr) = stream.into_split();
    let mut reader = BufReader::new(rd);
    let mut wr = wr;

    // 1) 브라우저: 전체 facts 페이지네이션 (offset/limit 왕복)
    wr.write_all(b"{\"method\":\"memory_browse\",\"params\":{\"offset\":0,\"limit\":1}}\n").await.unwrap();
    let evs = read_events(&mut reader, "MemoryData").await;
    match evs.last() {
        Some(Event::MemoryData { facts, total }) => {
            assert_eq!(facts.as_slice(), ["caspar는 한국어를 쓴다".to_string()].as_slice(), "첫 페이지: {facts:?}");
            assert_eq!(*total, 2, "total은 필터 없는 전체 수");
        }
        other => panic!("MemoryData 미수신: {other:?}"),
    }
    wr.write_all(b"{\"method\":\"memory_browse\",\"params\":{\"offset\":1,\"limit\":1}}\n").await.unwrap();
    let evs = read_events(&mut reader, "MemoryData").await;
    match evs.last() {
        Some(Event::MemoryData { facts, total }) => {
            assert_eq!(facts.as_slice(), ["선호 에디터는 vim".to_string()].as_slice(), "둘째 페이지: {facts:?}");
            assert_eq!(*total, 2);
        }
        other => panic!("MemoryData 미수신: {other:?}"),
    }

    // 2) 통계: 총 fact 수 / 세션 수 / 결정 수
    wr.write_all(b"{\"method\":\"memory_stats\"}\n").await.unwrap();
    let evs = read_events(&mut reader, "MemoryStats").await;
    assert!(matches!(evs.last(), Some(Event::MemoryStats { facts: 2, sessions: 1, decisions: 1 })), "MemoryStats 왕복: {evs:?}");

    // 3) 삭제 → 확인 응답(갱신 total) → 재검색해서 실제로 사라졌는지 확인
    wr.write_all("{\"method\":\"memory_delete\",\"params\":{\"content\":\"caspar는 한국어를 쓴다\"}}\n".as_bytes()).await.unwrap();
    let evs = read_events(&mut reader, "MemoryData").await;
    assert!(matches!(evs.last(), Some(Event::MemoryData { facts, total }) if facts.is_empty() && *total == 1), "삭제 확인: {evs:?}");
    wr.write_all(b"{\"method\":\"memory_browse\",\"params\":{\"offset\":0,\"limit\":10}}\n").await.unwrap();
    let evs = read_events(&mut reader, "MemoryData").await;
    assert!(matches!(evs.last(), Some(Event::MemoryData { facts, total }) if facts.len() == 1 && facts[0] == "선호 에디터는 vim" && *total == 1), "삭제 후 재검색: {evs:?}");

    // 4) 없는 fact 삭제 — 무음 드롭 금지, 명시적 오류
    wr.write_all("{\"method\":\"memory_delete\",\"params\":{\"content\":\"없는 fact\"}}\n".as_bytes()).await.unwrap();
    let evs = read_events(&mut reader, "Error").await;
    assert!(matches!(evs.last(), Some(Event::Error { message, .. }) if message.contains("삭제할 fact가 없음")), "명시적 오류: {evs:?}");

    // 5) 세션 요약 조회 — 저장된 요약 반환 / 없는 세션은 빈 문자열 (사이드바 표시 계약)
    wr.write_all(b"{\"method\":\"summary_get\",\"params\":{\"session\":\"s0\"}}\n").await.unwrap();
    let evs = read_events(&mut reader, "SummaryData").await;
    assert!(matches!(evs.last(), Some(Event::SummaryData { session, summary }) if session == "s0" && summary == "다운로드 폴더 정리를 마쳤고 vim 설정을 바꿨다"), "요약 왕복: {evs:?}");
    wr.write_all(b"{\"method\":\"summary_get\",\"params\":{\"session\":\"no-such\"}}\n").await.unwrap();
    let evs = read_events(&mut reader, "SummaryData").await;
    assert!(matches!(evs.last(), Some(Event::SummaryData { summary, .. }) if summary.is_empty()), "요약 없음 = 빈 문자열: {evs:?}");
}

#[tokio::test]
async fn session_summarized_after_idle_and_injected_into_next_session_prompt() {
    let root = tmp_root("summarize_idle");
    let systems = Arc::new(Mutex::new(Vec::new()));
    let provider = Capturing { systems: systems.clone() };
    let paths = Paths { data_dir: root.join("data"), config_dir: root.join("config") };
    let mut d = Daemon::new(Box::new(provider), paths);
    d.summary_idle = Duration::from_millis(150); // 테스트용 단축 (기본 30초)
    let d = Arc::new(d);
    tokio::spawn(d.clone().serve(root.join("d.sock")));
    let mut stream = None;
    for _ in 0..20 {
        if let Ok(s) = tokio::net::UnixStream::connect(root.join("d.sock")).await { stream = Some(s); break; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let stream = stream.expect("데몬 소켓 연결 실패");
    let (rd, wr) = stream.into_split();
    let mut reader = BufReader::new(rd);
    let mut wr = wr;

    // s1: 한 턴 대화
    wr.write_all(b"{\"method\":\"session_create\",\"params\":{\"id\":\"s1\"}}\n").await.unwrap();
    wr.write_all("{\"method\":\"message_send\",\"params\":{\"session\":\"s1\",\"text\":\"다운로드 폴더 정리해줘\"}}\n".as_bytes()).await.unwrap();
    let _ = read_events(&mut reader, "StreamDelta").await; // 턴 완료 대기

    // 유휴(150ms) 경과 + 폴링(50ms) 후 요약 저장 — 최대 3초 재시도
    let db = root.join("data/memory.db");
    let mut saved = None;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Ok(s) = MemoryStore::open(&db) {
            if let Some(first) = s.search_summaries("정리").unwrap().first() {
                saved = Some(first.clone());
                break;
            }
        }
    }
    let saved = saved.expect("유휴 경과 후 세션 요약이 summaries에 저장되어야 함");

    // s2 시작 → 첫 턴 시스템 프롬프트에 '이전 세션 요약' + 저장된 요약 포함
    wr.write_all(b"{\"method\":\"session_create\",\"params\":{\"id\":\"s2\"}}\n").await.unwrap();
    wr.write_all("{\"method\":\"message_send\",\"params\":{\"session\":\"s2\",\"text\":\"새 작업 시작\"}}\n".as_bytes()).await.unwrap();
    let _ = read_events(&mut reader, "StreamDelta").await;

    let chat_prompts: Vec<String> = systems.lock().iter().filter(|s| !s.contains("요약 도우미")).cloned().collect();
    assert!(chat_prompts.len() >= 2, "채팅 호출 2회 이상이어야 함: {chat_prompts:?}");
    assert!(!chat_prompts[0].contains("이전 세션 요약"), "s1 프롬프트에는 요약이 없어야 함");
    assert!(chat_prompts[1].contains("## 이전 세션 요약"), "s2 프롬프트에 요약 섹션 필요: {}", chat_prompts[1]);
    assert!(chat_prompts[1].contains(&saved), "s2 프롬프트가 저장된 요약을 포함해야 함: {saved}");
}

#[tokio::test]
async fn new_session_start_forces_pending_summary_without_idle_wait() {
    let root = tmp_root("summarize_force");
    let systems = Arc::new(Mutex::new(Vec::new()));
    let provider = Capturing { systems };
    let paths = Paths { data_dir: root.join("data"), config_dir: root.join("config") };
    let mut d = Daemon::new(Box::new(provider), paths);
    d.summary_idle = Duration::from_secs(60); // 타이머 경로 배제 — 오직 '새 세션 시작' 트리거 검증
    let d = Arc::new(d);
    tokio::spawn(d.clone().serve(root.join("d.sock")));
    let mut stream = None;
    for _ in 0..20 {
        if let Ok(s) = tokio::net::UnixStream::connect(root.join("d.sock")).await { stream = Some(s); break; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let stream = stream.expect("데몬 소켓 연결 실패");
    let (rd, wr) = stream.into_split();
    let mut reader = BufReader::new(rd);
    let mut wr = wr;

    // s1 대화 — 유휴(60s)는 절대 도래하지 않음
    wr.write_all(b"{\"method\":\"session_create\",\"params\":{\"id\":\"s1\"}}\n").await.unwrap();
    wr.write_all("{\"method\":\"message_send\",\"params\":{\"session\":\"s1\",\"text\":\"vim 설정 바꿔줘\"}}\n".as_bytes()).await.unwrap();
    let _ = read_events(&mut reader, "StreamDelta").await;

    // 새 세션 s2 생성 → s1 즉시 요약 (SessionCreate 트리거)
    wr.write_all(b"{\"method\":\"session_create\",\"params\":{\"id\":\"s2\"}}\n").await.unwrap();
    let _ = read_events(&mut reader, "ModeChanged").await;

    let db = root.join("data/memory.db");
    let mut found = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Ok(s) = MemoryStore::open(&db) {
            if s.search_summaries("vim").unwrap().len() > 0 { found = true; break; }
        }
    }
    assert!(found, "새 세션 시작이 유휴 대기 없이 s1 요약을 강제 트리거해야 함");
}
