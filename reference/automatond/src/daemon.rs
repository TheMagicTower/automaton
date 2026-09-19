//! 참조 데몬 (§2·§4) — UDS에서 ndjson RPC 서빙. 연결당 1개 writer 태스크(mpsc)가
//! 감사 로그(§5) + Apprentice 결정 기록·힌트 주입(§6) + 소켓 출력을 단일 책임으로 수행한다.

use automaton_apprentice::Apprentice;
use automaton_core::{AgentLoop, ApprovalGate, ApprovalOutcome, Provider};
use automaton_memory::MemoryStore;
use automaton_policy::{Engine, Rule, VerdictTemplate};
use automaton_proto::{ActionInfo, Decision, Event, Mode, Request};
use parking_lot::Mutex; // 즉시 unwrap 락 — rs-parking-lot 룰(가드 직접 반환). async 채널은 tokio::sync 유지
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};

pub struct Paths {
    pub data_dir: PathBuf,    // ~/.local/share/automaton
    pub config_dir: PathBuf,  // ~/.config/automaton
}

impl Paths {
    pub fn default_dirs() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        Paths {
            data_dir: PathBuf::from(&home).join(".local/share/automaton"),
            config_dir: PathBuf::from(&home).join(".config/automaton"),
        }
    }
    pub fn policy(&self) -> PathBuf { self.config_dir.join("policy.toml") }
    pub fn memory(&self) -> PathBuf { self.data_dir.join("memory.db") }
    pub fn audit(&self, session: &str) -> PathBuf { self.data_dir.join("audit").join(format!("{session}.jsonl")) }
}

pub struct Daemon {
    pub provider: Box<dyn Provider>,
    pub paths: Paths,
    pub gate: Arc<SessionGate>,
    engine: Mutex<Engine>,
    apprentice: Apprentice,
    store: MemoryStore,
    sessions: Mutex<HashMap<String, Mode>>,
    /// 승인 id → (툴, 타깃) — "항상 허용" 스코프·팬텀 id 차단(§5) + 저널 target 공급(§6, 리뷰 반영)
    pending_asks: Mutex<HashMap<String, (String, String)>>,
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
        Daemon { provider, paths, gate, engine: Mutex::new(engine), apprentice, store, sessions: Mutex::new(HashMap::new()), pending_asks: Mutex::new(HashMap::new()), approval_seq: std::sync::atomic::AtomicU64::new(1) }
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
                let _ = tx.send(Event::Error { session: None, message: "요청 파싱 실패".into() });
                continue;
            };
            match req {
                Request::SessionCreate { id } => {
                    self.sessions.lock().insert(id.clone(), Mode::Chat);
                    let _ = tx.send(Event::ModeChanged { session: id, mode: Mode::Chat });
                }
                Request::ModeSwitch { session, to } => {
                    // §5 모드 전환 승인은 프로토콜 계약 — 셸이 승인 배너로 사용자 확인 후에만 이 요청을 보낸다.
                    self.sessions.lock().insert(session.clone(), to);
                    let _ = tx.send(Event::ModeChanged { session, mode: to });
                }
                Request::ApprovalRespond { session, approval, decision, always } => {
                    // 승인 id 검증 + 툴 스코프 + 결정 저널(§5·§6) — 응답 시점에 실제 target으로 기록 (리뷰 반영: FTS는 target만 색인)
                    let entry = self.pending_asks.lock().remove(&approval);
                    match entry {
                        Some((tool, target)) => {
                            let approved = decision == Decision::Approve;
                            let _ = self.apprentice.note_decision(&session, &ActionInfo { tool: tool.clone(), target: target.clone(), risk: String::new() }, "ask", if approved { "approve" } else { "deny" });
                            if always && approved { // 거절+항상허용 조합은 Allow 발행 금지 (프로토콜 수비, 리뷰 반영)
                                if tool == "shell.exec" {
                                    // 보안 불변식: shell.exec에 대한 툴 단위 무제한 '항상 허용'은 금지 (임의 셸 명령 실행 위험 차단)
                                    eprintln!("보안 경고: shell.exec는 영구 '항상 허용' 규칙으로 등록 불가 (매회 승인 필요)");
                                } else {
                                    let mut e = self.engine.lock();
                                    e.grant_always(Rule { name: format!("granted-{approval}-{tool}"), tool: Some(tool.clone()), app: None, category: None, verdict: VerdictTemplate::Allow });
                                    let _ = e.save(&self.paths.policy());
                                }
                            }
                            if let Some(tx) = self.gate.pending.lock().remove(&tool) {
                                let _ = tx.send(decision);
                            }
                        }
                        None => {
                            let _ = tx.send(Event::Error { session: Some(session), message: format!("발행되지 않은 승인 id: {approval}") });
                        }
                    }
                }
                Request::MessageSend { session, text } => {
                    let d = self.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move { d.run_session(&session, &text, tx).await });
                }
                Request::SessionList => {
                    // M1 미구현 — 무음 드롭 금지, 명시적 오류 응답
                    let _ = tx.send(Event::Error { session: None, message: "SessionList는 M1 미구현".into() });
                }
            }
        }
    }

    /// 연결당 단일 출력 루프: 힌트 주입 → 감사(최종 형태) → 소켓 출력
    async fn writer_loop(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<Event>, mut wr: tokio::net::unix::OwnedWriteHalf) {
        use std::io::Write;
        while let Some(mut ev) = rx.recv().await {
            let session = session_of(&ev);
            // 1) Apprentice 힌트 주입 (§6) — ASK에 과거 유사 승인 부착 + 승인 id 등록
            if let Event::ApprovalRequested { ref mut approval, ref mut action, hint: ref mut hint @ None, .. } = ev {
                // 승인 id를 데몬 전역 유니크로 재발급 — 루프의 approval_seq가 턴마다 1로 재시작해 낡은 id 충돌 방지 (리뷰 자문)
                *approval = format!("a{}", self.approval_seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
                if let Some(h) = self.apprentice.hint_for(action).ok().flatten() {
                    *hint = Some(h); // 힌트는 별도 필드로만 전달 — risk 변형 시 배너 이중 표시 (리뷰 자문)
                }
                self.pending_asks.lock().insert(approval.clone(), (action.tool.clone(), action.target.clone()));
            }
            // 2) 감사 로그(§5) — 힌트 부착된 최종 형태 기록 (사후 분석 일관성)
            if let Some(s) = &session {
                if let Ok(line) = serde_json::to_string(&ev) {
                    let path = self.paths.audit(s);
                    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).ok(); }
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                        let _ = writeln!(f, "{line}");
                    }
                }
            }
            // 3) 소켓 출력
            if let Ok(line) = serde_json::to_string(&ev) {
                let _ = wr.write_all(line.as_bytes()).await;
                let _ = wr.write_all(b"\n").await;
            }
        }
    }

    async fn run_session(self: Arc<Self>, session: &str, text: &str, tx: mpsc::UnboundedSender<Event>) {
        let mode = *self.sessions.lock().get(session).unwrap_or(&Mode::Chat);
        let registry = match mode { Mode::Mac => automaton_tools::Registry::mac_set(), _ => automaton_tools::Registry::coding_set() };
        let engine = self.engine.lock().clone();
        // &dyn Provider 전달 — Chunk 2의 &P 포워딩 구현 사용 (Box 소유권 유지, 실측 E0277 반영)
        let loop_ = AgentLoop::new(&*self.provider, self.gate.clone(), engine, registry, mode);
        let mut history = self.store.messages(session).unwrap_or_default();
        let d = self.clone(); // 3단계 초안 발행용 Arc 사본 — emit 클로저로 이동
        let mut emit = move |e: Event| {
            let _ = tx.send(e.clone());
            // 3단계(§6) — 승인 배너 직후 답변 초안 발행. 같은 채널로 직렬화되어
            // ApprovalRequested → DraftSuggestions 순서가 보장된다. 초안은 칩일 뿐 승인 아님.
            if let Event::ApprovalRequested { ref session, ref action, .. } = e {
                let drafts = d.apprentice.drafts_for(action).unwrap_or_default();
                if !drafts.is_empty() {
                    let _ = tx.send(Event::DraftSuggestions { session: session.clone(), suggestions: drafts });
                }
            }
        }; // Send 클로저 — run_turn의 + Send 바운드 충족(실측 반영)
        let prior = history.len(); // 신규 분절만 저장 — 기존 재기록 시 매 턴 중복 증식(실측 결함)
        if let Err(e) = loop_.run_turn(session, &mut history, text.to_string(), &mut emit).await {
            let _ = emit(Event::Error { session: Some(session.to_string()), message: format!("턴 실패: {e}") }); // 침묵 끊김 방지 (리뷰 자문)
        }
        for m in &history[prior..] { let _ = self.store.append_message(session, &m.role, &m.content); }
    }
}

fn session_of(ev: &Event) -> Option<String> {
    match ev {
        Event::StreamDelta { session, .. } | Event::ToolStarted { session, .. } | Event::ToolResult { session, .. }
        | Event::ApprovalRequested { session, .. } | Event::DraftSuggestions { session, .. } | Event::ModeChanged { session, .. } => Some(session.clone()),
        Event::Error { session, .. } => session.clone(),
    }
}

/// 세션당 직렬 승인 게이트 — ASK 시 oneshot 등록 후 ApprovalRespond 대기.
pub struct SessionGate {
    pub pending: Mutex<HashMap<String, oneshot::Sender<Decision>>>,
}

impl SessionGate {
    pub fn new() -> Self {
        SessionGate { pending: Mutex::new(HashMap::new()) }
    }
}

impl Default for SessionGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ApprovalGate for SessionGate {
    async fn decide(&self, action: ActionInfo) -> ApprovalOutcome {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().insert(action.tool, tx);
        match rx.await {
            Ok(Decision::Approve) => ApprovalOutcome::Approve,
            _ => ApprovalOutcome::Deny,
        }
    }
}
