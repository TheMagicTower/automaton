import Foundation

enum Mode: String, Codable, CaseIterable, Sendable { case code, mac, chat }

struct ActionInfo: Codable, Sendable { let tool: String; let target: String; let risk: String }
struct Hint: Codable, Sendable {
    let text: String
    let similarCount: UInt
    enum CodingKeys: String, CodingKey { case text; case similarCount = "similar_count" }
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
        case "error": self = .error(session: try c.decodeIfPresent(String.self, forKey: .session), message: try c.decode(String.self, forKey: .message))
        case "usage": self = .usage(session: try c.decode(String.self, forKey: .session))
        case "summary_data": self = .summaryData(session: try c.decode(String.self, forKey: .session), summary: try c.decode(String.self, forKey: .summary))
        case "memory_data": self = .memoryData(session: try c.decodeIfPresent(String.self, forKey: .session), facts: try c.decode([String].self, forKey: .facts), total: try c.decode(Int.self, forKey: .total))
        case "memory_stats": self = .memoryStats(session: try c.decodeIfPresent(String.self, forKey: .session), facts: try c.decode(Int.self, forKey: .facts), sessions: try c.decode(Int.self, forKey: .sessions), decisions: try c.decode(Int.self, forKey: .decisions))
        default: throw DecodingError.dataCorruptedError(forKey: .type, in: c, debugDescription: "unknown: \(type)")
        }
    }
    func encode(to encoder: Encoder) throws { throw EncodingError.invalidValue(self, .init(codingPath: [], debugDescription: "recv-only")) }
}

/// POSIX 소켓 데몬 연결 — 실제 OS Thread에서 블로킹 I/O 수행.
/// Swift Task 협력 스레드는 블로킹 read에서 문제를 일으키므로 Thread 사용.
actor DaemonConnection {
    private let socketPath: String
    private var continuation: AsyncStream<ShellEvent>.Continuation?
    private var writeFD: Int32 = -1
    private var pendingSends: [String] = []

    init(socketPath: String = NSString(string: "~/.local/share/automaton/automatond.sock").expandingTildeInPath) {
        self.socketPath = socketPath
    }

    func events() -> AsyncStream<ShellEvent> {
        AsyncStream { cont in
            self.continuation = cont
            self.startThread()
        }
    }

    private func startThread() {
        let path = socketPath
        Thread.detachNewThread { [weak self] in
            guard let self else { return }
            self.posixReadLoop(path: path, conn: self)
        }
    }

    /// 실제 OS 스레드에서 실행 — 블로킹 recv 안전
    private nonisolated func posixReadLoop(path: String, conn: DaemonConnection) {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        guard bytes.count < 104 else { close(fd); return }
        withUnsafeMutableBytes(of: &addr.sun_path) { $0.copyBytes(from: bytes) }
        let rc = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sa in
                connect(fd, sa, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard rc == 0 else { close(fd); return }

        FileHandle.standardError.write(Data("[automaton] POSIX connected fd=\(fd)\n".utf8))

        // Actor에 fd 설정 + 큐 플러시 (동기 대기)
        let sem = DispatchSemaphore(value: 0)
        Task { await conn.setFDAndFlush(fd); sem.signal() }
        sem.wait()

        FileHandle.standardError.write(Data("[automaton] READ LOOP start fd=\(fd)\n".utf8))

        var buffer = Data()
        var buf = [UInt8](repeating: 0, count: 65536)
        while true {
            let n = recv(fd, &buf, buf.count, 0)
            if n <= 0 {
                FileHandle.standardError.write(Data("[automaton] recv EOF/err errno=\(errno)\n".utf8))
                break
            }
            buffer.append(contentsOf: buf[0..<n])
            FileHandle.standardError.write(Data("[automaton] RECV \(n) bytes\n".utf8))

            // 개행 단위 이벤트 파싱
            while let nl = buffer.firstIndex(of: 0x0A) {
                let line = String(data: Data(buffer[buffer.startIndex..<nl]), encoding: .utf8) ?? ""
                buffer = Data(buffer[buffer.index(after: nl)...])
                guard !line.isEmpty else { continue }

                if let data = line.data(using: .utf8),
                   let ev = try? JSONDecoder().decode(ShellEvent.self, from: data) {
                    Task { await conn.yieldEvent(ev) }
                }
            }
        }

        close(fd)
        Task { await conn.disconnected() }
    }

    // Actor 메서드들

    func setFDAndFlush(_ fd: Int32) {
        writeFD = fd
        for json in pendingSends { rawWrite(json) }
        pendingSends.removeAll()
    }

    func yieldEvent(_ ev: ShellEvent) {
        continuation?.yield(ev)
    }

    func disconnected() {
        continuation?.finish()
        continuation = nil
        writeFD = -1
    }

    private func rawWrite(_ json: String) {
        guard writeFD >= 0, let data = (json + "\n").data(using: .utf8) else { return }
        data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            var sent = 0
            while sent < raw.count {
                let n = write(writeFD, raw.baseAddress!.advanced(by: sent), raw.count - sent)
                if n <= 0 { break }
                sent += n
            }
        }
    }
    func send(_ json: String) {
        FileHandle.standardError.write(Data("[automaton] SEND: fd=\(writeFD) \(json.prefix(50))\n".utf8))
        guard writeFD >= 0 else { pendingSends.append(json); return }
        rawWrite(json)
    }

    func sendRequest(method: String, params: [String: Any]) {
        let payload: [String: Any] = params.isEmpty ? ["method": method] : ["method": method, "params": params]
        if let data = try? JSONSerialization.data(withJSONObject: payload),
           let s = String(data: data, encoding: .utf8) {
            send(s)
        }
    }
}
