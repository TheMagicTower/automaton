import Foundation
import Network

enum Mode: String, Codable, CaseIterable, Sendable { case code, mac, chat }

struct ActionInfo: Codable, Sendable { let tool: String; let target: String; let risk: String }
struct Hint: Codable, Sendable {
    let text: String
    let similarCount: UInt
    enum CodingKeys: String, CodingKey { case text; case similarCount = "similar_count" } // 와이어 키는 serde 스네이크케이스 (실측 결함)
}

enum ShellEvent: Codable, Sendable {
    case streamDelta(session: String, delta: String)
    case toolStarted(session: String, tool: String, summary: String)
    case toolResult(session: String, tool: String, ok: Bool, summary: String)
    case approvalRequested(session: String, approval: String, action: ActionInfo, hint: Hint?)
    case draftSuggestions(session: String, suggestions: [String])
    case modeChanged(session: String, mode: Mode)
    case error(session: String?, message: String)
    case usage(session: String)
    case summaryData(session: String, summary: String)
    case memoryData(session: String?, facts: [String], total: Int)
    case memoryStats(session: String?, facts: Int, sessions: Int, decisions: Int)

    enum CodingKeys: String, CodingKey { case type, session, delta, tool, summary, ok, approval, action, hint, mode, message, suggestions, usage, facts, total, sessions, decisions }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let type = try c.decode(String.self, forKey: .type)
        switch type {
        case "stream_delta": self = .streamDelta(session: try c.decode(String.self, forKey: .session), delta: try c.decode(String.self, forKey: .delta))
        case "tool_started": self = .toolStarted(session: try c.decode(String.self, forKey: .session), tool: try c.decode(String.self, forKey: .tool), summary: try c.decode(String.self, forKey: .summary))
        case "tool_result": self = .toolResult(session: try c.decode(String.self, forKey: .session), tool: try c.decode(String.self, forKey: .tool), ok: try c.decode(Bool.self, forKey: .ok), summary: try c.decode(String.self, forKey: .summary))
        case "approval_requested": self = .approvalRequested(session: try c.decode(String.self, forKey: .session), approval: try c.decode(String.self, forKey: .approval), action: try c.decode(ActionInfo.self, forKey: .action), hint: try c.decodeIfPresent(Hint.self, forKey: .hint))
        case "draft_suggestions": self = .draftSuggestions(session: try c.decode(String.self, forKey: .session), suggestions: try c.decode([String].self, forKey: .suggestions))
        case "mode_changed": self = .modeChanged(session: try c.decode(String.self, forKey: .session), mode: try c.decode(Mode.self, forKey: .mode))
        case "usage": self = .usage(session: try c.decode(String.self, forKey: .session))
        case "summary_data": self = .summaryData(session: try c.decode(String.self, forKey: .session), summary: try c.decode(String.self, forKey: .summary))
        case "memory_data": self = .memoryData(session: try c.decodeIfPresent(String.self, forKey: .session), facts: try c.decode([String].self, forKey: .facts), total: try c.decode(Int.self, forKey: .total))
        case "memory_stats": self = .memoryStats(session: try c.decodeIfPresent(String.self, forKey: .session), facts: try c.decode(Int.self, forKey: .facts), sessions: try c.decode(Int.self, forKey: .sessions), decisions: try c.decode(Int.self, forKey: .decisions))
        default: throw DecodingError.dataCorruptedError(forKey: .type, in: c, debugDescription: "알 수 없는 이벤트: \(type)")
        }
    }
    func encode(to encoder: Encoder) throws { throw EncodingError.invalidValue(self, .init(codingPath: [], debugDescription: "수신 전용")) }
}

/// 데몬 연결 — actor 직렬화, ndjson 한 줄씩 송수신.
actor DaemonConnection {
    private var connection: NWConnection?
    private let socketPath: String
    private var continuation: AsyncStream<ShellEvent>.Continuation?
    private var buffer = Data()
    private var ready = false
    private var pendingSends: [String] = [] // 연결 수립 전 송신 큐 — 첫 session_create 유실 방지 (리뷰 이슈 ②)

    init(socketPath: String = NSString(string: "~/.local/share/automaton/automatond.sock").expandingTildeInPath) {
        self.socketPath = socketPath
    }

    func events() -> AsyncStream<ShellEvent> {
        AsyncStream { cont in
            self.continuation = cont
            self.connect()
        }
    }

    private func connect() {
        let conn = NWConnection(to: .unix(path: socketPath), using: .tcp) // .unix(path:) — unixPath 아님 (실측)
        connection = conn
        conn.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready: Task { await self?.flushPending() }
            case .failed: Task { await self?.handleDisconnect() }
            default: break
            }
        }
        receiveLoop(conn)
        conn.start(queue: .global(qos: .userInitiated))
    }

    private func flushPending() {
        ready = true
        for json in pendingSends { send(json) }
        pendingSends.removeAll()
    }

    private func handleDisconnect() {
        ready = false // 재접속 대비 큐 의미 보존 (리뷰 자문)
        continuation?.finish()
        continuation = nil
        connection = nil
    }

    private func receiveLoop(_ conn: NWConnection) {
        conn.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, done, error in
            guard let self else { return }
            if let data { Task { await self.consume(data) } }
            if error != nil || done { Task { await self.handleDisconnect() } } // clean EOF도 종료 처리 — 무한 대기 방지 (리뷰 자문)
            else { Task { await self.receiveLoop(conn) } } // #ActorIsolatedCall 경고 방지 (실측)
        }
    }

    private func consume(_ data: Data) {
        buffer.append(data)
        while let nl = buffer.firstIndex(of: 0x0A) {
            let line = Data(buffer[buffer.startIndex..<nl])
            buffer = buffer[buffer.index(after: nl)...]
            guard let ev = try? JSONDecoder().decode(ShellEvent.self, from: line) else { continue }
            continuation?.yield(ev)
        }
    }

    func send(_ json: String) {
        guard ready else { pendingSends.append(json); return } // ready 전 송신은 큐잉 — 조용한 드롭 방지
        guard let conn = connection, let data = (json + "\n").data(using: .utf8) else { return }
        conn.send(content: data, completion: .contentProcessed { _ in })
    }
    func sendRequest(method: String, params: [String: Any]) {
        // params 없는 요청(memory_stats 등 unit variant)은 params 키 자체를 실어 보내지 않는다 —
        // 데몬 serde(tag+content)는 unit variant의 비어 있지 않은 params를 거부한다.
        let payload: [String: Any] = params.isEmpty ? ["method": method] : ["method": method, "params": params]
        if let data = try? JSONSerialization.data(withJSONObject: payload),
           let s = String(data: data, encoding: .utf8) { send(s) }
    }
}
