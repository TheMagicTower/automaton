import AppKit
import SwiftUI

@main
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

// MARK: - ShellModel (기존 로직 유지)

@Observable
@MainActor
final class ShellModel {
    var mode: Mode = .chat
    var stream: [String] = []
    var pendingApproval: (id: String, action: ActionInfo, hint: Hint?)?
    var draftSuggestions: [String] = []
    var connected = false
    var memoryCount: Int = 0
    let voiceOutput = VoiceOutputManager()
    private let conn = DaemonConnection()
    private var session = "shell-\(UInt64(Date().timeIntervalSince1970))"
    private var started = false

    /// 새 세션 — 기존 대화 스트림 클리어 + 새 세션 ID로 재연결
    func newSession() {
        stream = []
        pendingApproval = nil
        draftSuggestions = []
        session = "shell-\(UInt64(Date().timeIntervalSince1970))"
        Task {
            await conn.sendRequest(method: "session_create", params: ["id": session])
        }
    }

    /// 메모리 조회 — 에이전트가 알고 있는 사용자 정보 표시
    func queryMemory() {
        stream.append("\n🧠 기억 조회 중...")
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
            let stream = await conn.events()
            await conn.sendRequest(method: "session_create", params: ["id": session])
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
        case .approvalRequested(_, let id, let action, let hint): pendingApproval = (id, action, hint); draftSuggestions = []
        case .draftSuggestions(_, let suggestions): draftSuggestions = suggestions
        case .modeChanged(_, let mode): self.mode = mode
        case .error(_, let message): stream.append("\n⚠ \(message)")
        }
    }

    func send(_ text: String) {
        voiceOutput.interrupt()
        stream.append("\n▸ \(text)")
        Task { await conn.sendRequest(method: "message_send", params: ["session": session, "text": text]) }
    }

    func requestMode(_ to: Mode) {
        Task { await conn.sendRequest(method: "mode_switch", params: ["session": session, "to": to.rawValue]) }
    }

    func respond(approve: Bool, always: Bool) {
        guard let p = pendingApproval else { return }
        Task { await conn.sendRequest(method: "approval_respond", params: ["session": session, "approval": p.id, "decision": approve ? "approve" : "deny", "always": always]) }
        pendingApproval = nil
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

    @FocusState private var inputFocused: Bool

    private var inputBar: some View {
        HStack {
            TextField("명령…", text: $draft).textFieldStyle(.plain).foregroundStyle(Theme.ivory)
                .focused($inputFocused)
                .onSubmit { if !draft.isEmpty { model.send(draft); draft = "" } }
            Button { if !draft.isEmpty { model.send(draft); draft = "" } } label: { Image(systemName: "paperplane.fill").foregroundStyle(Theme.gold) }.buttonStyle(.plain)
        }.padding(10)
        .onAppear { inputFocused = true }
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
