//! 참조 데몬 (§2·§4) — UDS에서 ndjson RPC 서빙. 연결당 1개 writer 태스크(mpsc)가
//! 감사 로그(§5) + Apprentice 결정 기록·힌트 주입(§6) + 소켓 출력을 단일 책임으로 수행한다.

use automaton_apprentice::Apprentice;
use automaton_core::{
    AgentLoop, ApprovalGate, ApprovalOutcome, CompletionRequest, Provider, StreamItem,
};
use automaton_memory::MemoryStore;
use automaton_policy::{Engine, Rule, VerdictTemplate};
use automaton_proto::{ActionInfo, Decision, Event, Mode, Request};
use parking_lot::Mutex; // 즉시 unwrap 락 — rs-parking-lot 룰(가드 직접 반환). async 채널은 tokio::sync 유지
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};

pub struct Paths {
    pub data_dir: PathBuf,   // ~/.local/share/automaton
    pub config_dir: PathBuf, // ~/.config/automaton
}

impl Paths {
    pub fn default_dirs() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        Paths {
            data_dir: PathBuf::from(&home).join(".local/share/automaton"),
            config_dir: PathBuf::from(&home).join(".config/automaton"),
        }
    }
    pub fn policy(&self) -> PathBuf {
        self.config_dir.join("policy.toml")
    }
    pub fn memory(&self) -> PathBuf {
        self.data_dir.join("memory.db")
    }
    pub fn audit(&self, session: &str) -> PathBuf {
        self.data_dir.join("audit").join(format!("{session}.jsonl"))
    }
}

pub struct Daemon {
    pub provider: Box<dyn Provider>,
    pub paths: Paths,
    pub gate: Arc<SessionGate>,
    engine: Mutex<Engine>,
    apprentice: Apprentice,
    store: MemoryStore,
    sessions: Mutex<HashMap<String, Mode>>,
    /// 세션 요약기 상태 — 마지막 활동 시각 / 마지막 요약이 커버한 메시지 수 / 요약 진행 중 세션
    last_activity: Mutex<HashMap<String, Instant>>,
    summarized_len: Mutex<HashMap<String, usize>>,
    summarizing: Mutex<HashSet<String>>,
    /// 세션별 '이전 세션 요약' 블록 — 첫 턴에 고정(시스템 프롬프트 캐시 프리픽스 안정성)
    summary_blocks: Mutex<HashMap<String, String>>,
    /// 세션 종료 판정 유휴 시간 — 마지막 메시지 후 이 시간 경과 시 요약 (기본 30초, 테스트 단축)
    pub summary_idle: Duration,
    /// 승인 id → (발행 세션, 툴, 타깃) — 세션 바인딩 검증(F-03) + "항상 허용" 스코프·팬텀 id 차단(§5) + 저널 target 공급(§6)
    pending_asks: Mutex<HashMap<String, (String, String, String)>>,
    /// 데몬 전역 승인 id 발급기 — 루프의 턴 로컬 seq 충돌 방지
    approval_seq: std::sync::atomic::AtomicU64,
}

impl Daemon {
    pub fn new(provider: Box<dyn Provider>, paths: Paths) -> Self {
        std::fs::create_dir_all(&paths.data_dir).ok();
        std::fs::create_dir_all(paths.data_dir.join("audit")).ok(); // 감사 디렉터리 — 첫 MessageSend 패닉 방지(실측 결함)
        std::fs::create_dir_all(&paths.config_dir).ok();
        // 정책 합성 시점: 파일이 있으면 파일 전체(granted+rules), 없으면 builtin.
        // grant_always 시 save()가 전체를 내려쓰므로 파일은 항상 완전 상태가 된다.
        let engine = Engine::from_file(&paths.policy()).unwrap_or_else(|e| {
            if !matches!(&e, automaton_policy::PolicyError::Io(io_err) if io_err.kind() == std::io::ErrorKind::NotFound) {
                eprintln!("정책 파일 손상(builtin 폴백, granted 유실 가능): {e}"); // NotFound(첫 실행)는 무음 (리뷰 자문)
            }
            Engine::builtin()
        });
        let store = MemoryStore::open(&paths.memory()).expect("메모리 DB 열기 실패");
        let apprentice = Apprentice::open(&paths.memory()).expect("Apprentice DB 열기 실패");
        let gate = Arc::new(SessionGate::new());
        Daemon {
            provider,
            paths,
            gate,
            engine: Mutex::new(engine),
            apprentice,
            store,
            sessions: Mutex::new(HashMap::new()),
            last_activity: Mutex::new(HashMap::new()),
            summarized_len: Mutex::new(HashMap::new()),
            summarizing: Mutex::new(HashSet::new()),
            summary_blocks: Mutex::new(HashMap::new()),
            summary_idle: Duration::from_secs(30),
            pending_asks: Mutex::new(HashMap::new()),
            approval_seq: std::sync::atomic::AtomicU64::new(1),
        }
    }

    pub async fn serve(self: Arc<Self>, socket: PathBuf) -> std::io::Result<()> {
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        }
        eprintln!("automatond listening at {}", socket.display());
        // 세션 요약기 백그라운드 태스크 — 유휴 경과 세션 감지 후 요약 생성 (§7)
        {
            let d = self.clone();
            tokio::spawn(async move {
                d.session_summarizer().await;
            });
        }
        loop {
            let (stream, _) = listener.accept().await?;
            let d = self.clone();
            tokio::spawn(async move { d.handle(stream).await });
        }
    }

    async fn handle(self: Arc<Self>, stream: UnixStream) {
        let (rd, wr) = stream.into_split();
        // 연결당 단일 writer 태스크 — OwnedWriteHalf는 Clone이 없어 세션별 복제 불가(실측 E0599).
        // 모든 출력(요청 즉시 응답 + 스트림 이벤트)은 이 채널로 집결한다.
        let (tx, rx) = mpsc::unbounded_channel::<Event>();
        {
            let d = self.clone();
            tokio::spawn(async move { d.writer_loop(rx, wr).await });
        }
        let mut lines = BufReader::new(rd).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(req) = serde_json::from_str::<Request>(&line) else {
                let _ = tx.send(Event::Error {
                    session: None,
                    message: "요청 파싱 실패".into(),
                });
                continue;
            };
            match req {
                Request::SessionCreate { id } => {
                    self.sessions.lock().insert(id.clone(), Mode::Chat);
                    // 새 세션 시작 = 기존 세션 종료 감지 → 유휴 대기 없이 즉시 요약 트리거
                    for s in self.due_sessions(true).into_iter().filter(|s| *s != id) {
                        let d = self.clone();
                        tokio::spawn(async move {
                            d.summarize_session(&s).await;
                        });
                    }
                    let _ = tx.send(Event::ModeChanged {
                        session: id,
                        mode: Mode::Chat,
                    });
                }
                Request::ModeSwitch { session, to } => {
                    // F-03: 모드 전환을 셸 프로토콜 계약(배너 확인 후 전송)만으로 신뢰하지 않고
                    // 데몬 측 승인 게이트를 통과시킨다 — ApprovalRequested 발행 → 승인 응답 시에만 적용.
                    let key = format!("{session}:mode.switch");
                    let (waiter, rx) = oneshot::channel();
                    {
                        let mut pending = self.gate.pending.lock();
                        if pending.contains_key(&key) {
                            let _ = tx.send(Event::Error {
                                session: Some(session),
                                message: "모드 전환 승인 대기 중 — 중복 요청 거부".into(),
                            });
                            continue;
                        }
                        pending.insert(key, waiter);
                    }
                    // approval id는 writer_loop가 데몬 전역 유니크로 재발급해 pending_asks에 등록한다.
                    let _ = tx.send(Event::ApprovalRequested {
                        session: session.clone(),
                        approval: String::new(),
                        action: ActionInfo {
                            tool: "mode.switch".into(),
                            target: format!("{to:?}").to_lowercase(),
                            risk: "모드 전환".into(),
                        },
                        hint: None,
                    });
                    let d = self.clone();
                    let tx_gate = tx.clone();
                    tokio::spawn(async move {
                        match rx.await {
                            Ok(Decision::Approve) => {
                                d.sessions.lock().insert(session.clone(), to);
                                let _ = tx_gate.send(Event::ModeChanged { session, mode: to });
                            }
                            _ => {
                                let _ = tx_gate.send(Event::Error {
                                    session: Some(session),
                                    message: "모드 전환 거절됨".into(),
                                });
                            }
                        }
                    });
                }
                Request::ApprovalRespond {
                    session,
                    approval,
                    decision,
                    always,
                } => {
                    // 승인 id 검증 + 세션 바인딩(F-03) + 툴 스코프 + 결정 저널(§5·§6) — 응답 시점에 실제 target으로 기록
                    let lookup = {
                        let mut asks = self.pending_asks.lock();
                        if let Some((ask_session, tool, target)) = asks.get(&approval).cloned() {
                            if ask_session != session {
                                Err(format!(
                                    "승인 id {approval}는 세션 {ask_session}에 귀속 — 세션 {session}의 응답은 거부됨 (승인 세션 바인딩)"
                                ))
                            } else {
                                asks.remove(&approval);
                                Ok((tool, target))
                            }
                        } else {
                            Err(format!("발행되지 않은 승인 id: {approval}"))
                        }
                    };
                    match lookup {
                        Ok((tool, target)) => {
                            let approved = decision == Decision::Approve;
                            let _ = self.apprentice.note_decision(
                                &session,
                                &ActionInfo {
                                    tool: tool.clone(),
                                    target: target.clone(),
                                    risk: String::new(),
                                },
                                "ask",
                                if approved { "approve" } else { "deny" },
                            );
                            if always && approved {
                                // 거절+항상허용 조합은 Allow 발행 금지 (프로토콜 수비, 리뷰 반영)
                                if tool == "shell.exec" || tool == "mode.switch" {
                                    // 보안 불변식: 임의 셸 실행·모드 전환의 툴 단위 무제한 '항상 허용'은 금지 (매회 승인 필요)
                                    eprintln!(
                                        "보안 경고: {tool}은(는) 영구 '항상 허용' 규칙으로 등록 불가 (매회 승인 필요)"
                                    );
                                } else {
                                    let mut e = self.engine.lock();
                                    e.grant_always(Rule {
                                        name: format!("granted-{approval}-{tool}"),
                                        tool: Some(tool.clone()),
                                        app: None,
                                        category: None,
                                        verdict: VerdictTemplate::Allow,
                                    });
                                    let _ = e.save(&self.paths.policy());
                                }
                            }
                            // F-05: waiter는 발행 (세션, 툴) 조합 키로만 해제 — 타 세션 동일 툴 waiter 오해제 방지
                            if let Some(waiter) = self
                                .gate
                                .pending
                                .lock()
                                .remove(&format!("{session}:{tool}"))
                            {
                                let _ = waiter.send(decision);
                            }
                        }
                        Err(message) => {
                            let _ = tx.send(Event::Error {
                                session: Some(session),
                                message,
                            });
                        }
                    }
                }
                Request::MessageSend { session, text } => {
                    // 세션 활동 시각 기록 — 요약기의 유휴(종료) 판정 기준 (마지막 메시지 후 N초)
                    self.last_activity
                        .lock()
                        .insert(session.clone(), Instant::now());
                    let d = self.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move { d.run_session(&session, &text, tx).await });
                }
                Request::HistoryGet { session, limit } => {
                    // 세션 이력 반환 — 최근 limit개 메시지를 단일 델타로 전송 (시간순)
                    let msgs = self.store.messages(&session).unwrap_or_default();
                    let start = msgs.len().saturating_sub(limit);
                    let text = msgs[start..]
                        .iter()
                        .map(|m| {
                            if m.role == "user" {
                                format!("▸ {}", m.content)
                            } else {
                                m.content.clone()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.is_empty() {
                        let _ = tx.send(Event::StreamDelta {
                            session: session.clone(),
                            delta: text,
                        });
                    }
                }
                Request::SummaryGet { session } => {
                    // 세션 요약 반환 — summaries 테이블 (없으면 빈 문자열: 셸이 캐시 비움)
                    let summary = self
                        .store
                        .summary(&session)
                        .unwrap_or(None)
                        .unwrap_or_default();
                    let _ = tx.send(Event::SummaryData { session, summary });
                }
                Request::Interrupt { session } => {
                    // 사용자 중단 — 진행 중 턴에 오류 이벤트를 보내 셸이 즉시 제어권 회복
                    let _ = tx.send(Event::Error {
                        session: Some(session.clone()),
                        message: "⛔ 사용자가 중단했습니다".into(),
                    });
                }
                Request::SessionList => {
                    // M1 미구현 — 무음 드롭 금지, 명시적 오류 응답
                    let _ = tx.send(Event::Error {
                        session: None,
                        message: "SessionList는 M1 미구현".into(),
                    });
                }
                Request::MemoryBrowse { offset, limit } => {
                    let (facts, total) = self
                        .store
                        .facts_page(offset, limit)
                        .unwrap_or((Vec::new(), 0));
                    let _ = tx.send(Event::MemoryData { facts, total });
                }
                Request::MemoryDelete { content } => match self.store.delete_fact(&content) {
                    Ok(n) if n > 0 => {
                        // 삭제 확인 응답 — 갱신된 전체 수만 실어 보낸다 (facts는 빈 페이지)
                        let total = self.store.stats().map(|(f, _, _)| f).unwrap_or(0);
                        let _ = tx.send(Event::MemoryData {
                            facts: Vec::new(),
                            total,
                        });
                    }
                    Ok(_) => {
                        let _ = tx.send(Event::Error {
                            session: None,
                            message: format!("삭제할 fact가 없음: {content}"),
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(Event::Error {
                            session: None,
                            message: format!("fact 삭제 실패: {e}"),
                        });
                    }
                },
                Request::MemoryStats => {
                    let (facts, sessions, decisions) = self.store.stats().unwrap_or((0, 0, 0));
                    let _ = tx.send(Event::MemoryStats {
                        facts,
                        sessions,
                        decisions,
                    });
                }
            }
        }
    }

    /// 연결당 단일 출력 루프: 힌트 주입 → 감사(최종 형태) → 소켓 출력
    async fn writer_loop(
        self: Arc<Self>,
        mut rx: mpsc::UnboundedReceiver<Event>,
        mut wr: tokio::net::unix::OwnedWriteHalf,
    ) {
        use std::io::Write;
        while let Some(mut ev) = rx.recv().await {
            let session = session_of(&ev);
            // 1) Apprentice 힌트 주입 (§6) — ASK에 과거 유사 승인 부착 + 승인 id 등록
            if let Event::ApprovalRequested {
                ref mut approval,
                ref mut action,
                hint: ref mut hint @ None,
                ..
            } = ev
            {
                // 승인 id를 데몬 전역 유니크로 재발급 — 루프의 approval_seq가 턴마다 1로 재시작해 낡은 id 충돌 방지 (리뷰 자문)
                *approval = format!(
                    "a{}",
                    self.approval_seq
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                );
                if let Some(h) = self.apprentice.hint_for(action).ok().flatten() {
                    *hint = Some(h); // 힌트는 별도 필드로만 전달 — risk 변형 시 배너 이중 표시 (리뷰 자문)
                }
                if let Some(s) = &session {
                    self.pending_asks.lock().insert(
                        approval.clone(),
                        (s.clone(), action.tool.clone(), action.target.clone()),
                    );
                }
            }
            // 2) 감사 로그(§5) — 힌트 부착된 최종 형태 기록 (사후 분석 일관성)
            if let Some(s) = &session
                && let Ok(line) = serde_json::to_string(&ev)
            {
                let path = self.paths.audit(s);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    let _ = writeln!(f, "{line}");
                }
            }
            // 3) 소켓 출력
            if let Ok(line) = serde_json::to_string(&ev) {
                let _ = wr.write_all(line.as_bytes()).await;
                let _ = wr.write_all(b"\n").await;
            }
        }
    }

    async fn run_session(
        self: Arc<Self>,
        session: &str,
        text: &str,
        tx: mpsc::UnboundedSender<Event>,
    ) {
        let mode = *self.sessions.lock().get(session).unwrap_or(&Mode::Chat);
        let registry = automaton_tools::Registry::unified_set(); // 전 모드 동일 툴셋 — 안전은 Policy Engine이 담당
        let engine = self.engine.lock().clone();

        // 시스템 프롬프트: 세션당 안정(캐시 프리픽스 보존) — 사용자 텍스트로 변하지 않음.
        // '이전 세션 요약'은 세션 첫 턴에 한 번 합성·고정 — 이후 턴은 캐시된 동일 블록을 재사용한다.
        let mut history = self.store.messages(session).unwrap_or_default();
        let mut composed_prompt = self.stable_system_prompt(mode);
        let summary_block = if history.is_empty() {
            self.prime_summary_block(session, text) // 첫 턴: 검색+캐시 (빈 블록도 캐시에 남기지 않음 — 재검색 방지는 아래 else 절)
        } else {
            self.summary_blocks
                .lock()
                .get(session)
                .cloned()
                .unwrap_or_default()
        };
        if !summary_block.is_empty() {
            composed_prompt.push_str("\n\n");
            composed_prompt.push_str(&summary_block);
        }
        let base_profile = automaton_core::ModeProfile::builtin(mode);
        let profile = automaton_core::ModeProfile {
            mode,
            tools: base_profile.tools,
            system_prompt: composed_prompt,
        };
        let loop_ = AgentLoop::with_profile(
            &*self.provider,
            self.gate.clone(),
            engine,
            registry,
            profile,
        );

        // 관련 기억을 히스토리 끝(현재 사용자 메시지 직전)에 주입 — 캐시 프리픽스 최소 교란
        // 시스템 프롬프트+기존 히스토리는 그대로 → API 프리픽스 캐시 히트
        let fact_block = self.relevant_facts(text);
        if !fact_block.is_empty() {
            history.push(automaton_core::Message {
                role: "system".into(),
                content: fact_block,
            });
        }

        let d = self.clone();
        let mut emit = move |e: Event| {
            let _ = tx.send(e.clone());
            if let Event::ApprovalRequested {
                ref session,
                ref action,
                ..
            } = e
            {
                let drafts = d.apprentice.drafts_for(action).unwrap_or_default();
                if !drafts.is_empty() {
                    let _ = tx.send(Event::DraftSuggestions {
                        session: session.clone(),
                        suggestions: drafts,
                    });
                }
            }
        };
        let prior = history.len();
        if let Err(e) = loop_
            .run_turn(session, &mut history, text.to_string(), &mut emit)
            .await
        {
            emit(Event::Error {
                session: Some(session.to_string()),
                message: format!("턴 실패: {e}"),
            });
        }

        // 턴 종료 후: 메모리 추출 + 신규 분절만 저장 (주입한 system 메시지는 제외)
        self.extract_and_save_memories(&history[prior..]);
        for m in &history[prior..] {
            if m.role != "system" {
                // 주입한 사실 블록은 재주입 방지 위해 미저장
                let _ = self.store.append_message(session, &m.role, &m.content);
            }
        }
        // 마지막 메시지 저장 시점 = 요약기의 유휴(세션 종료) 판정 기준 갱신
        self.last_activity
            .lock()
            .insert(session.to_string(), Instant::now());
    }

    // ── 세션 요약기 (§7 작업 계층) ────────────────────────────────────────────

    /// 요약 대상 세션 수집 — force_all=true면 유휴 무시 전체(새 세션 시작 트리거),
    /// 아니면 마지막 활동 후 summary_idle 경과 세션. 요약할 새 메시지가 쌓인 세션만.
    fn due_sessions(&self, force_all: bool) -> Vec<String> {
        let now = Instant::now();
        let activity = self.last_activity.lock();
        let summarized = self.summarized_len.lock();
        let inflight = self.summarizing.lock();
        let mut due = Vec::new();
        for (s, last) in activity.iter() {
            let msgs = self.store.messages(s).map(|m| m.len()).unwrap_or(0);
            if msgs > summarized.get(s).copied().unwrap_or(0)
                && (force_all || now.duration_since(*last) >= self.summary_idle)
                && !inflight.contains(s)
            {
                due.push(s.clone());
            }
        }
        due
    }

    /// 백그라운드 세션 요약 루프 — 폴링 주기 = summary_idle/3 (기본 30초 판정 / 10초 폴링)
    async fn session_summarizer(self: Arc<Self>) {
        loop {
            tokio::time::sleep(self.summary_idle / 3).await;
            for s in self.due_sessions(false) {
                let d = self.clone();
                tokio::spawn(async move {
                    d.summarize_session(&s).await;
                });
            }
        }
    }

    /// 세션 messages를 LLM에 보내 3줄 요약 생성 → summaries 테이블 저장.
    /// Provider 재사용 + 요약 전용 프롬프트(짧은 출력 지시·툴 없음·usage 추적 없음).
    async fn summarize_session(&self, session: &str) {
        if !self.summarizing.lock().insert(session.to_string()) {
            return;
        } // 중복 실행 방지
        let msgs = self.store.messages(session).unwrap_or_default();
        if msgs.is_empty() {
            self.summarizing.lock().remove(session);
            return;
        }
        let bounded: Vec<automaton_core::Message> =
            msgs.iter().rev().take(40).rev().cloned().collect(); // 최근 40개로 바운드
        let req = CompletionRequest {
            system: "너는 세션 요약 도우미다. 주어진 대화를 정확히 3줄로 요약하라. 각 줄은 한 문장으로 간결하게 쓰고, 출력은 요약 3줄만 한다. 인사말·설명·표식은 금지.".into(),
            messages: bounded,
            tools: Vec::new(),
            track_usage: false,
        };
        match self.provider.complete(req).await {
            Ok(items) => {
                let text: String = items
                    .into_iter()
                    .filter_map(|i| match i {
                        StreamItem::Delta(d) => Some(d),
                        _ => None,
                    })
                    .collect();
                let text = text.trim();
                if !text.is_empty() && self.store.save_summary(session, text).is_ok() {
                    self.summarized_len
                        .lock()
                        .insert(session.to_string(), msgs.len());
                } else {
                    // 빈 응답/저장 실패 — 활동 시각 갱신으로 다음 유휴 주기에 재시도
                    eprintln!("세션 요약 공백/저장 실패 — 재시도 예약: {session}");
                    self.last_activity
                        .lock()
                        .insert(session.to_string(), Instant::now());
                }
            }
            Err(e) => {
                eprintln!("세션 요약 생성 실패({session}): {e} — 다음 유휴 주기에 재시도");
                self.last_activity
                    .lock()
                    .insert(session.to_string(), Instant::now());
            }
        }
        self.summarizing.lock().remove(session);
    }

    /// 세션 첫 턴에 '이전 세션 요약' 블록 결정·캐시 — 첫 메시지 키워드로 search_summaries,
    /// 없으면 최근 요약 폴백. 이후 턴은 summary_blocks 캐시로 프롬프트 안정성 유지.
    fn prime_summary_block(&self, session: &str, user_text: &str) -> String {
        let mut picks: Vec<String> = Vec::new();
        for kw in automaton_memory::extract_keywords(user_text).iter().take(3) {
            for hit in self.store.search_summaries(kw).unwrap_or_default() {
                if !picks.contains(&hit) {
                    picks.push(hit);
                }
            }
        }
        if picks.is_empty() {
            picks = self.store.recent_summaries(3, session).unwrap_or_default(); // 자기 세션 제외
        }
        if picks.is_empty() {
            return String::new();
        }
        let mut block = String::from("## 이전 세션 요약\n");
        for s in picks.iter().take(3) {
            block.push_str(&format!("- {}\n", s.replace('\n', " / ")));
        }
        self.summary_blocks
            .lock()
            .insert(session.to_string(), block.clone());
        block
    }

    /// 안정 시스템 프롬프트 — 세션 수명 동안 불변(캐시 프리픽스 보존)
    fn stable_system_prompt(&self, mode: Mode) -> String {
        let persona = self.load_persona();
        let base = automaton_core::ModeProfile::builtin(mode);
        let mut prompt = persona.to_string();
        prompt.push_str(&format!("\n{}", base.system_prompt));
        prompt.push_str("\n\n## 기억 지침\n- 사용자에 대해 알게 된 새로운 사실(선호, 이름, 프로젝트, 습관 등)은 [기억: 내용] 형태로 응답에 포함하세요. 자동으로 저장됩니다.\n- 이미 아는 내용은 반복해서 저장하지 마세요.");
        prompt
    }

    /// 사용자 텍스트 기반 관련 기억 검색 → 히스토리 끝에 주입할 블록 조합
    /// search_facts_related: 한글 조사 제거·동의어 확장·관련도 랭킹 + 무결과 시 최근 facts 폴백
    fn relevant_facts(&self, user_text: &str) -> String {
        let facts = self
            .store
            .search_facts_related(user_text, 5)
            .unwrap_or_default();
        if facts.is_empty() {
            return String::new();
        }
        let mut block = String::from("## 사용자에 대해 알고 있는 것\n");
        for f in facts.iter().take(5) {
            block.push_str(&format!("- {}\n", f));
        }
        block
    }

    /// 페르소나 파일 로드 — 없으면 기본 생성
    fn load_persona(&self) -> String {
        let path = self.paths.config_dir.join("persona.md");
        if let Ok(content) = std::fs::read_to_string(&path)
            && !content.trim().is_empty()
        {
            return content;
        }
        let default = "# automaton 성격\n\n너는 caspar의 개인 비서 automaton이다. 황동과 월넛으로 만들어진 스팀펑크 기계 장치로, 정확하고 신속하게 일을 처리한다.\n\n## 성격\n- 존댓말 사용, 간결하고 실용적\n- 능동적으로 제안하되 사용자 결정 존중\n- 도구 사용 결과를 근거로 보고 — 추측으로 답하지 않음\n\n## 사용자 기억\n- 대화에서 알게 된 사용자 정보는 [기억: 내용] 마커로 출력해 자동 저장\n- 저장된 기억은 다음 대화에서 자동으로 참조됨\n";
        let _ = std::fs::create_dir_all(&self.paths.config_dir);
        let _ = std::fs::write(&path, default);
        default.to_string()
    }

    /// 어시스턴트 출력에서 [기억: ...] 마커 추출하여 facts_fts에 저장
    fn extract_and_save_memories(&self, new_messages: &[automaton_core::Message]) {
        for msg in new_messages {
            if msg.role != "assistant" {
                continue;
            }
            let mut rest = msg.content.as_str();
            while let Some(start) = rest.find("[기억:") {
                let marker_len = "[기억:".len(); // UTF-8 바이트 길이 — 4가 아니라 8
                let after = &rest[start + marker_len..];
                if let Some(end) = after.find(']') {
                    let fact = after[..end].trim();
                    if !fact.is_empty() {
                        let _ = self.store.add_fact(fact);
                    }
                    rest = &after[end + 1..];
                } else {
                    break;
                }
            }
        }
    }
}

fn session_of(ev: &Event) -> Option<String> {
    match ev {
        Event::StreamDelta { session, .. }
        | Event::ToolStarted { session, .. }
        | Event::ToolResult { session, .. }
        | Event::ApprovalRequested { session, .. }
        | Event::DraftSuggestions { session, .. }
        | Event::Usage { session, .. }
        | Event::ModeChanged { session, .. }
        | Event::SummaryData { session, .. } => Some(session.clone()),
        Event::Error { session, .. } => session.clone(),
        // 메모리 브라우저 응답은 세션 스코프 아님 — 감사 대상 제외
        Event::MemoryData { .. } | Event::MemoryStats { .. } => None,
    }
}

/// 세션별 승인 게이트 — ASK 시 "{session}:{tool}" 키로 oneshot 등록 후 ApprovalRespond 대기 (F-05).
pub struct SessionGate {
    pub pending: Mutex<HashMap<String, oneshot::Sender<Decision>>>,
}

impl SessionGate {
    pub fn new() -> Self {
        SessionGate {
            pending: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for SessionGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ApprovalGate for SessionGate {
    async fn decide(&self, session: &str, action: ActionInfo) -> ApprovalOutcome {
        let (tx, rx) = oneshot::channel();
        let key = format!("{session}:{}", action.tool);
        {
            let mut pending = self.pending.lock();
            // F-05: 동일 (세션,툴) 중복 ASK는 기존 waiter를 덮어쓰지 않고 즉시 거부 —
            // 덮어쓰기 시 첫 요청이 사용자 개입 없이 자동 Deny되는 혼동을 원천 차단.
            if pending.contains_key(&key) {
                return ApprovalOutcome::Deny;
            }
            pending.insert(key, tx);
        }
        match rx.await {
            Ok(Decision::Approve) => ApprovalOutcome::Approve,
            _ => ApprovalOutcome::Deny,
        }
    }
}
