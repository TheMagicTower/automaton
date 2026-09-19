import SwiftUI

@main
struct AutomatonApp: App {
    @State private var model = ShellModel()
    @State private var voiceInput = VoiceInputManager()
    var body: some Scene {
        MenuBarExtra("automaton", systemImage: "gearshape.fill") {
            ShellView().environment(model).environment(voiceInput).frame(width: 380, height: 480)
        }
        .menuBarExtraStyle(.window)
    }
}

@Observable
@MainActor
final class ShellModel {
    var mode: Mode = .chat
    var stream: [String] = []
    var pendingApproval: (id: String, action: ActionInfo, hint: Hint?)?
    var draftSuggestions: [String] = []
    var connected = false
    let voiceOutput = VoiceOutputManager() // §12 1단계 TTS — streamDelta 구독
    private let conn = DaemonConnection()
    private let session = "shell-\(UInt64(Date().timeIntervalSince1970))"
    private var started = false

    func start() {
        guard !started else { return }
        started = true
        Task {
            let stream = await conn.events() // 연결 수립을 먼저 확정
            await conn.sendRequest(method: "session_create", params: ["id": session]) // 송신은 ready 전 큐잉됨
            for await ev in stream {
                connected = true
                apply(ev)
            }
            connected = false
        }
    }

    private func apply(_ ev: ShellEvent) {
        switch ev {
        case .streamDelta(_, let delta): stream.append(delta); voiceOutput.append(delta: delta)
        case .toolStarted(_, let tool, _): stream.append("\n⚙ \(tool)"); voiceOutput.flush()
        case .toolResult(_, let tool, let ok, let summary): stream.append(ok ? " ✓ \(tool)" : " ✗ \(tool): \(summary)")
        case .approvalRequested(_, let id, let action, let hint): pendingApproval = (id, action, hint); draftSuggestions = [] // 새 배너에는 낡은 칩 없음
        case .draftSuggestions(_, let suggestions): draftSuggestions = suggestions
        case .modeChanged(_, let mode): self.mode = mode
        case .error(_, let message): stream.append("\n⚠ \(message)")
        }
    }

    func send(_ text: String) {
        voiceOutput.interrupt() // 새 입력 → 음성 재생 즉시 중지
        stream.append("\n▸ \(text)")
        Task { await conn.sendRequest(method: "message_send", params: ["session": session, "text": text]) }
    }

    /// §5 모드 전환 승인 — 확인 대화상자 후에만 전송 (프로토콜 계약)
    func requestMode(_ to: Mode) {
        Task { await conn.sendRequest(method: "mode_switch", params: ["session": session, "to": to.rawValue]) }
    }

    func respond(approve: Bool, always: Bool) {
        guard let p = pendingApproval else { return }
        Task { await conn.sendRequest(method: "approval_respond", params: ["session": session, "approval": p.id, "decision": approve ? "approve" : "deny", "always": always]) }
        pendingApproval = nil
        draftSuggestions = []
    }
}

struct ShellView: View {
    @Environment(ShellModel.self) private var model
    @Environment(VoiceInputManager.self) private var voiceInput
    @State private var draft = ""
    @State private var confirmMode: Mode?

    var body: some View {
        VStack(spacing: 0) {
            modeBar
            Divider().overlay(Theme.brass.opacity(0.4))
            ScrollViewReader { proxy in
                ScrollView {
                    Text(model.stream.joined()).font(.system(size: 12, design: .monospaced)).foregroundStyle(Theme.ivory).frame(maxWidth: .infinity, alignment: .leading).id("bottom")
                }.onChange(of: model.stream.count) { proxy.scrollTo("bottom") }
            }
            if let p = model.pendingApproval {
                ApprovalBanner(action: p.action, hint: p.hint) { ok, always in model.respond(approve: ok, always: always) }
                if !model.draftSuggestions.isEmpty { draftChips }
            }
            inputBar
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

    private var modeBar: some View {
        HStack(spacing: 8) {
            Theme.title("automaton")
            Spacer()
            ForEach(Mode.allCases, id: \.self) { m in
                Button { confirmMode = m } label: {
                    Text(m == .code ? "⚙ code" : m == .mac ? "🔭 mac" : "📖 chat")
                        .font(.system(size: 12, design: .serif)).padding(.horizontal, 10).padding(.vertical, 3)
                        .background(Capsule().fill(model.mode == m ? AnyShapeStyle(LinearGradient(colors: [Theme.gold.opacity(0.9), Theme.brass], startPoint: .top, endPoint: .bottom)) : AnyShapeStyle(Color.clear)))
                        .overlay(Capsule().strokeBorder(model.mode == m ? Theme.gold : Theme.dim.opacity(0.6)))
                        .foregroundStyle(model.mode == m ? Theme.walnut : Theme.dim)
                }.buttonStyle(.plain)
            }
            voiceStatus
            voiceToggle
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
        .help(voiceInput.statusNote ?? (voiceInput.enabled ? "Push-to-Talk: Cmd+Shift+Space 길게 눌러 말하기" : "음성 입력 켜기"))
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

    private var inputBar: some View {
        HStack {
            TextField("명령…", text: $draft).textFieldStyle(.plain).foregroundStyle(Theme.ivory)
                .onSubmit { if !draft.isEmpty { model.send(draft); draft = "" } }
            Button { if !draft.isEmpty { model.send(draft); draft = "" } } label: { Image(systemName: "paperplane.fill").foregroundStyle(Theme.gold) }.buttonStyle(.plain)
        }.padding(10)
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
