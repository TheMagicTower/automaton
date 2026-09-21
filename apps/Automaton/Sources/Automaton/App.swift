import AppKit
import SwiftUI

/// SPM 실행 파일용 진입점 — 번들 없이 NSApplication을 수동 구성한다.
/// @main + NSApplicationDelegate는 NSApplicationMain을 호출하는데,
/// SPM 실행 파일에는 Info.plist/번들 컨텍스트가 없어 메뉴바 등록이 실패한다.
/// 수동 런루프 + accessory 정책이 SPM 메뉴바 앱의 표준 패턴이다.
@main
enum AutomatonEntry {
    /// NSApplication.delegate는 weak 참조 — 지역변수가 조기 해제되면
    /// MenuBarController와 NSStatusItem이 함께 해제되어 메뉴바 아이콘이 사라진다.
    /// static 강한 참조로 생명주기를 앱 수명과 일치시킨다.
    @MainActor private static var retained: AnyObject?

    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory) // 메뉴바 전용, Dock 아이콘 없음

        let delegate = AutomatonAppDelegate()
        AutomatonEntry.retained = delegate // strong ref — delegate 조기 해제 방지
        app.delegate = delegate

        app.run()
    }
}

@MainActor
final class AutomatonAppDelegate: NSObject, NSApplicationDelegate {
    private var menuBar = MenuBarController()
    private let model = ShellModel()
    private let voiceInput = VoiceInputManager()

    func applicationDidFinishLaunching(_ notification: Notification) {
        // MenuBarExtra 대신 NSStatusItem + NSPopover 사용
        // (키보드 입력 무반응 + 클릭 시 자동 닫힘 결함 해결)
        menuBar.setup(model: model, voiceInput: voiceInput)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false // 메뉴바 앱은 창이 없어도 유지
    }
}

// MARK: - ShellModel

@Observable
@MainActor
final class ShellModel {
    var mode: Mode = .chat
    var stream: [ChatEntry] = []
    var sessions: [SessionRecord] = []
    var pendingApproval: (id: String, action: ActionInfo, hint: Hint?)?
    var draftSuggestions: [String] = []
    var isThinking = false
    var connected = false
    // 기억 브라우저 상태 — memory_browse/memory_stats 응답 (우클릭 메뉴 → 팝오버)
    var memoryFacts: [String] = []
    var memoryTotal = 0
    var memoryStats: (facts: Int, sessions: Int, decisions: Int)?
    let voiceOutput = VoiceOutputManager()
    var voiceMuted: Bool {
        get { voiceOutput.isMuted }
        set { voiceOutput.isMuted = newValue }
    }
    private let conn = DaemonConnection()
    private var session: String {
        didSet { UserDefaults.standard.set(session, forKey: "automaton.session") }
    }
    private var started = false
    /// history_get 응답 대기 — 데몬은 이력을 "▸ " 접두 텍스트 blob 한 덩어리로 보낸다
    private var historyPending = false
    private var persistTask: Task<Void, Never>?
    /// 현재 세션 요약 스냅샷 — persistSessions가 stream으로 기록을 재생성할 때 반영(기록이 아직 없을 수 있음)
    private var currentSummary: String?

    var currentSessionID: String { session }

    init() {
        let saved = UserDefaults.standard.string(forKey: "automaton.session")
        self.session = saved ?? "shell-\(UInt64(Date().timeIntervalSince1970))"
        self.sessions = SessionStore.load()
        if let rec = sessions.first(where: { $0.id == session }) {
            self.stream = rec.entries
        }
        // 즉시 연결 — .onAppear 대기 없이 데몬 이벤트 수신 보장
        start()
    }

    /// 새 세션 — 기존 대화 스트림 클리어 + 새 세션 ID로 재연결
    func newSession() {
        flushSessions()
        session = "shell-\(UInt64(Date().timeIntervalSince1970))"
        stream = []
        pendingApproval = nil
        draftSuggestions = []
        isThinking = false
        historyPending = false
        Task {
            await conn.sendRequest(method: "session_create", params: ["id": session])
        }
    }

    /// 세션 전환 — 캐시된 기록 즉시 표시 후 데몬 history로 대체.
    /// session_create 미전송: 기존 세션의 모드를 Chat으로 덮어쓰는 부수효과 방지
    /// (데몬은 미등록 세션도 기본 Chat으로 실행 가능).
    func switchSession(_ id: String) {
        guard id != session else { return }
        flushSessions()
        session = id
        stream = sessions.first(where: { $0.id == id })?.entries ?? []
        pendingApproval = nil
        draftSuggestions = []
        isThinking = false
        historyPending = true
        Task {
            await conn.sendRequest(method: "history_get", params: ["session": session, "limit": 50])
            await conn.sendRequest(method: "summary_get", params: ["session": session]) // 사이드바 요약 — history와 함께
        }
    }

    /// 메모리 조회 — 에이전트가 알고 있는 사용자 정보 표시
    func queryMemory() {
        appendEntry(.tool, "⚙ 기억 조회 중…")
        historyPending = false // message_send 전송 = 라이브 턴 시작 — 대기 중 history 오인 방지 (send()와 동일 계약)
        Task {
            await conn.sendRequest(method: "message_send", params: [
                "session": session,
                "text": "지금까지 알게 된 나에 대한 정보를 모두 나열해주세요. 메모리에 저장된 사실만 기반으로 답하세요."
            ])
        }
    }

    func start() {
        guard !started else { return }
        started = true
        Task {
            let events = await conn.events()
            await conn.sendRequest(method: "session_create", params: ["id": session])
            // 이전 대화 이력 로딩 — 저장된 세션이면 최근 메시지 표시
            historyPending = true
            await conn.sendRequest(method: "history_get", params: ["session": session, "limit": 50])
            requestSessionSummaries() // 사이드바 전체 요약 로딩 — 현재 + 캐시된 세션
            for await ev in events {
                connected = true
                apply(ev)
            }
            connected = false
        }
    }

    /// 사이드바 요약 로딩 — 현재 + 캐시된 세션 전부 summary_get (데몬 summaries 테이블)
    private func requestSessionSummaries() {
        var seen = Set<String>()
        let ids = [session] + sessions.map(\.id)
        Task {
            for id in ids where seen.insert(id).inserted {
                await conn.sendRequest(method: "summary_get", params: ["session": id])
            }
        }
    }

    /// 기억 브라우저 — facts 첫 페이지 + 통계 조회 (팝오버 onAppear·새로고침)
    func browseMemory() {
        Task {
            await conn.sendRequest(method: "memory_browse", params: ["offset": 0, "limit": 100])
            await conn.sendRequest(method: "memory_stats", params: [:])
        }
    }

    /// fact 삭제 — memory_delete 후 재조회로 목록·통계 갱신 (요청은 연결당 순차 처리되어 반영 보장)
    func deleteMemory(_ fact: String) {
        Task {
            await conn.sendRequest(method: "memory_delete", params: ["content": fact])
            await conn.sendRequest(method: "memory_browse", params: ["offset": 0, "limit": 100])
            await conn.sendRequest(method: "memory_stats", params: [:])
        }
    }

    private func apply(_ ev: ShellEvent) {
        // 세션 요약은 사이드바 기록 갱신 — 타 세션 스트림 필터 대상 아님(전 세션 요약도 표시)
        if case .summaryData(let s, let summary) = ev {
            let value = summary.isEmpty ? nil : summary
            if s == session { currentSummary = value }
            if let i = sessions.firstIndex(where: { $0.id == s }) {
                sessions[i].summary = value
                SessionStore.save(sessions)
            }
            return
        }
        // 타 세션 이벤트 무시 — 전환 직후 구세션 스트림이 새 뷰에 섞이는 것 방지
        if let s = Self.sessionOf(ev), s != session { return }
        switch ev {
        case .streamDelta(_, let delta):
            isThinking = false
            if historyPending {
                // history_get 응답 = 구형 텍스트 blob → 구조화 변환해 캐시 대체.
                // 복원 이력은 TTS 발화 대상 아님(기존 startup 낭독 결함 제거).
                historyPending = false
                stream = ChatEntry.parseLegacy(delta)
                schedulePersist()
            } else {
                appendDelta(delta)
                voiceOutput.append(delta: delta) // 실시간 응답만 TTS — 이력 복원은 읽지 않음
            }
        case .toolStarted(_, _, let summary):
            isThinking = true // 툴 실행 중 인디케이터 — 셸 명령 등 장시간 실행 시각화
            appendEntry(.tool, "⚙ \(summary)")
            voiceOutput.flush()
        case .toolResult(_, let tool, let ok, let summary):
            isThinking = true // 턴 계속 진행 — 다음 툴/텍스트 대기
            appendEntry(.tool, ok ? "✓ \(summary)" : "✗ \(tool): \(summary)")
        case .approvalRequested(_, let id, let action, let hint):
            pendingApproval = (id, action, hint)
            draftSuggestions = []
        case .draftSuggestions(_, let suggestions):
            draftSuggestions = suggestions
        case .modeChanged(_, let mode):
            self.mode = mode
        case .summaryData:
            break // 상단 선처리(사이드바 기록 갱신) 후 도달 불가 — 스위치 완전성용
        case .usage:
            isThinking = false // 턴 종료 — usage 이벤트에서 확실히 해제
        case .memoryData(_, let facts, let total):
            memoryTotal = total
            // 빈 facts + 잔여 total = memory_delete 확인 응답(데몬 계약) → 목록 지우지 않음, 후속 browse가 갱신
            if !(facts.isEmpty && total > 0) { memoryFacts = facts }
        case .memoryStats(_, let facts, let sessions, let decisions):
            memoryStats = (facts, sessions, decisions)
        case .error(_, let message):
            isThinking = false
            appendEntry(.error, message)
        }
    }

    private static func sessionOf(_ ev: ShellEvent) -> String? {
        switch ev {
        case .streamDelta(let s, _), .toolStarted(let s, _, _), .toolResult(let s, _, _, _),
             .approvalRequested(let s, _, _, _), .draftSuggestions(let s, _), .modeChanged(let s, _),
             .usage(let s), .summaryData(let s, _):
            return s
        case .memoryData(let s, _, _), .memoryStats(let s, _, _, _):
            return s // nil 허용 — 기억 이벤트는 세션 무관(데몬이 session 필드 없이 발행)
        case .error(let s, _):
            return s
        }
    }

    /// 스트리밍 델타 — 마지막 에이전트 엔트리에 병합하되, 문단 경계(\n\n)에서 새 버블 분리.
    /// 하나의 버블에 셸 출력+분석이 몰리는 것을 방지 — 각 문단이 독립 버블이 됨.
    private func appendDelta(_ delta: String) {
        if let last = stream.last, last.role == .agent {
            stream[stream.count - 1].content += delta
            // 문단 분리 — 마지막 \n\n에서 잘라 새 버블 시작 (짧은 문단은 그대로 유지)
            let content = stream[stream.count - 1].content
            if content.contains("\n\n"), let range = content.range(of: "\n\n", options: .backwards) {
                let before = String(content[content.startIndex..<range.lowerBound])
                let after = String(content[range.upperBound...])
                if !after.isEmpty {
                    stream[stream.count - 1].content = before
                    stream.append(ChatEntry(role: .agent, content: after))
                }
            }
        } else {
            stream.append(ChatEntry(role: .agent, content: delta))
        }
        schedulePersist()
    }

    /// 신규 엔트리 — 부드러운 슬라이드업 트랜지션으로 등장
    func appendEntry(_ role: UserRole, _ content: String) { // internal — InputComposer interrupt에서 접근
        let entry = ChatEntry(role: role, content: content)
        stream.append(entry) // withAnimation 제거 — 지연 렌더링 방지, 사용자 메시지 즉시 표시
        schedulePersist()
    }

    func send(_ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        voiceOutput.interrupt()
        isThinking = true
        historyPending = false
        appendEntry(.user, trimmed)
        Task { await conn.sendRequest(method: "message_send", params: ["session": session, "text": trimmed]) }
    }

    func requestMode(_ to: Mode) {
        Task { await conn.sendRequest(method: "mode_switch", params: ["session": session, "to": to.rawValue]) }
    }

    func respond(approve: Bool, always: Bool) {
        guard let p = pendingApproval else { return }
        Task { await conn.sendRequest(method: "approval_respond", params: ["session": session, "approval": p.id, "decision": approve ? "approve" : "deny", "always": always]) }
        pendingApproval = nil
    }

    /// 현재 턴 중단 — 데몬에 interrupt 요청, UI 상태 즉시 리셋
    func interruptCurrent() async {
        await conn.sendRequest(method: "interrupt", params: ["session": session])
    }

    // MARK: - 세션 영속화 (UserDefaults)

    /// 토큰 단위 저장 부하 방지 — 변경 후 0.4초 디바운스 저장
    private func schedulePersist() {
        persistTask?.cancel()
        persistTask = Task {
            try? await Task.sleep(for: .milliseconds(400))
            guard !Task.isCancelled else { return }
            persistSessions()
        }
    }

    /// 즉시 저장 — 종료/세션 전환 직전 호출
    func flushSessions() {
        persistTask?.cancel()
        persistTask = nil
        persistSessions()
    }

    private func persistSessions() {
        var recs = sessions.filter { $0.id != session }
        if !stream.isEmpty {
            var entries = stream
            if entries.count > 200 { entries.removeFirst(entries.count - 200) } // 세션당 대화 캐시 상한
            recs.append(SessionRecord(id: session, entries: entries, updatedAt: Date(), summary: currentSummary))
        }
        recs.sort { $0.updatedAt > $1.updatedAt }
        if recs.count > 20 { recs.removeLast(recs.count - 20) } // 보관 세션 수 상한
        sessions = recs
        SessionStore.save(recs)
    }
}

// MARK: - ShellView

struct ShellView: View {
    @Environment(ShellModel.self) private var model
    @Environment(VoiceInputManager.self) private var voiceInput
    /// 창 모드에서만 세션 사이드바 표시 — 팝오버(400pt)에는 폭이 부족
    private let showSidebar: Bool
    @State private var draft = ""
    @State private var confirmMode: Mode?

    init(showSidebar: Bool = false) {
        self.showSidebar = showSidebar
    }

    private let bottomID = "bottom"

    var body: some View {
        HStack(spacing: 0) {
            if showSidebar {
                SessionSidebar()
                Divider().overlay(Theme.brass.opacity(0.4))
            }
            VStack(spacing: 0) {
                modeBar
                Divider().overlay(Theme.brass.opacity(0.4))
                transcript
                if let p = model.pendingApproval {
                    ApprovalBanner(action: p.action, hint: p.hint) { ok, always in model.respond(approve: ok, always: always) }
                    if !model.draftSuggestions.isEmpty { draftChips }
                }
                ComposerBar(draft: $draft)
            }
        }
        .background(Theme.walnut)
        .onAppear {
            model.start()
            let shell = model // 인스턴스 직접 캡처 — 뷰 구조체 탈출 캡처 회피
            voiceInput.onTransmit = { shell.send($0) }
            voiceInput.onListeningStart = { shell.voiceOutput.interrupt() }
            Task { await voiceInput.prepare() }
        }
        .confirmationDialog("모드를 전환할까요?", isPresented: Binding(get: { confirmMode != nil }, set: { if !$0 { confirmMode = nil } }), titleVisibility: .visible) {
            Button("전환") { if let m = confirmMode { model.requestMode(m) }; confirmMode = nil }
            Button("취소", role: .cancel) { confirmMode = nil }
        }
    }

    /// 대화 기록 — 역할별 버블 + 새 엔트리 슬라이드업 + 하단 자동 스크롤
    private var transcript: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 8) { // LazyVStack 금지 — NSHostingView(창 모드)에서 스크롤 크래시 유발
                    if model.stream.isEmpty && !model.isThinking { emptyState }
                    ForEach(model.stream) { entry in
                        ChatBubble(entry: entry)
                            .id(entry.id)
                            .transition(.asymmetric(
                                insertion: .move(edge: .bottom).combined(with: .opacity),
                                removal: .opacity
                            ))
                    }
                    if model.isThinking { thinkingIndicator } // 응답이 올 자리 = 사용자 메시지 바로 아래
                    Color.clear.frame(height: 1).id(bottomID)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            // 새 메시지(엔트리 추가) + 스트리밍(내용 증가) 모두 하단 유지 — 기존 동작 계승
            .onChange(of: model.stream.last?.id) { _, _ in proxy.scrollTo(bottomID, anchor: .bottom) }
            .onChange(of: model.stream.last?.content) { _, _ in proxy.scrollTo(bottomID, anchor: .bottom) }
            .onChange(of: model.isThinking) { _, on in
                if on { proxy.scrollTo(bottomID, anchor: .bottom) } // 로딩 시작 시에도 스크롤
            }
        }
    }

    /// 대화 없을 때 안내 — 첫 사용 진입 장벽 완화
    private var emptyState: some View {
        VStack(spacing: 10) {
            Image(systemName: "bubble.left.and.bubble.right")
                .font(.system(size: 28))
                .foregroundStyle(Theme.brass.opacity(0.6))
            Text("automaton에 메시지를 보내 대화를 시작하세요")
                .font(.system(size: 12, design: .serif))
                .foregroundStyle(Theme.dim)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 48)
        .padding(.bottom, 24)
    }

    /// 인라인 로딩 — 사용자 메시지 바로 아래, 응답이 올 자리에 표시
    private var thinkingIndicator: some View {
        HStack(spacing: 8) {
            TimelineView(.animation(minimumInterval: 1.0 / 30.0)) { timeline in
                let t = timeline.date.timeIntervalSinceReferenceDate
                let angle = (t.truncatingRemainder(dividingBy: 2.0)) / 2.0 * 360.0
                Image(systemName: "gearshape.fill")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Theme.brass)
                    .rotationEffect(.degrees(angle))
            }
            TimelineView(.animation(minimumInterval: 0.4)) { timeline in
                let phase = Int(timeline.date.timeIntervalSinceReferenceDate / 0.4) % 3
                HStack(spacing: 2) {
                    ForEach(0..<3, id: \.self) { i in
                        Circle()
                            .fill(i == phase ? Theme.gold : Theme.dim.opacity(0.3))
                            .frame(width: 3, height: 3)
                    }
                }
            }
        }
        .padding(.top, 2)
    }

    private var modeBar: some View {
        HStack(spacing: 6) {
            Theme.title("automaton")
            Spacer()
            ForEach(Mode.allCases, id: \.self) { m in
                Button { confirmMode = m } label: {
                    Text(m == .code ? "⚙" : m == .mac ? "🔭" : "📖")
                        .font(.system(size: 13))
                        .padding(.horizontal, 8).padding(.vertical, 3)
                        .background(Capsule().fill(model.mode == m ? AnyShapeStyle(LinearGradient(colors: [Theme.gold.opacity(0.9), Theme.brass], startPoint: .top, endPoint: .bottom)) : AnyShapeStyle(Color.clear)))
                        .overlay(Capsule().strokeBorder(model.mode == m ? Theme.gold : Theme.dim.opacity(0.6)))
                        .foregroundStyle(model.mode == m ? Theme.walnut : Theme.dim)
                }.buttonStyle(.plain)
            }
            // 새 세션 버튼
            Button { model.newSession() } label: {
                Image(systemName: "plus.message")
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.gold)
            }.buttonStyle(.plain).help("새 세션")

            // 메모리 조회 버튼
            Button { model.queryMemory() } label: {
                Image(systemName: "brain")
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.gold)
            }.buttonStyle(.plain).help("기억 조회")

            voiceStatus
            voiceToggle
            speakerToggle
        }.padding(10)
    }

    /// §12 1단계 상태 인디케이터 — 핫키 청취/인식 처리 중 표시
    private var voiceStatus: some View {
        Group {
            switch voiceInput.state {
            case .listening: Text("🔴 listening")
            case .processing: Text("🟡 processing")
            case .idle: EmptyView()
            }
        }
        .font(.system(size: 10, design: .monospaced))
        .foregroundStyle(Theme.ivory)
    }

    private var voiceToggle: some View {
        Button {
            voiceInput.setEnabled(!voiceInput.enabled)
        } label: {
            Image(systemName: voiceInput.enabled ? "mic.fill" : "mic.slash")
                .font(.system(size: 13))
                .foregroundStyle(voiceInput.enabled ? Theme.gold : Theme.dim)
        }
        .buttonStyle(.plain)
    }

    private var speakerToggle: some View {
        Button {
            model.voiceMuted.toggle()
        } label: {
            Image(systemName: model.voiceMuted ? "speaker.slash" : "speaker.wave.2")
                .font(.system(size: 13))
                .foregroundStyle(model.voiceMuted ? Theme.dim : Theme.gold)
        }
        .buttonStyle(.plain)
        .help(model.voiceMuted ? "음성 출력 켜기" : "음성 출력 끄기")
    }

    /// §6 3단계 답변 초안 칩 — 클릭하면 입력창에 해당 텍스트가 채워진다(전송은 사용자 몫).
    private var draftChips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(model.draftSuggestions, id: \.self) { s in
                    Button { draft = s } label: {
                        Text(s).font(.caption).padding(.horizontal, 10).padding(.vertical, 4)
                            .background(Capsule().fill(Theme.brass.opacity(0.15)))
                            .overlay(Capsule().strokeBorder(Theme.gold.opacity(0.6)))
                            .foregroundStyle(Theme.ivory)
                    }.buttonStyle(.plain)
                }
            }.padding(.horizontal, 10).padding(.bottom, 6)
        }
    }
}

struct ApprovalBanner: View {
    let action: ActionInfo
    let hint: Hint?
    let respond: (Bool, Bool) -> Void
    @State private var always = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Theme.title("⚖ \(action.tool)")
            Text(action.target.isEmpty ? action.risk : "\(action.target) — \(action.risk)").font(.caption).foregroundStyle(Theme.ivory)
            if let h = hint { Text("◈ \(h.text) (유사 \(h.similarCount)건)").font(.caption2).foregroundStyle(Theme.dim) }
            HStack {
                Toggle("항상 허용", isOn: $always).toggleStyle(.switch).controlSize(.mini).font(.caption2).foregroundStyle(Theme.dim)
                Spacer()
                Button("거절") { respond(false, false) }.buttonStyle(.bordered).tint(Theme.dim)
                Button("승인") { respond(true, always) }.buttonStyle(.borderedProminent).tint(Theme.brass)
            }
        }.padding(10).background(RoundedRectangle(cornerRadius: 8).fill(Theme.brass.opacity(0.10)).overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.brass)))
    }
}
