import AVFoundation
import Carbon.HIToolbox
import Observation
import Speech

/// §12 1단계 — 글로벌 핫키 Push-to-Talk. Cmd+Shift+Space 누르는 동안 ko-KR 인식(지원 시 온디바이스),
/// 떼면 최종 문장을 에이전트 루프(ShellModel.send)로 전송. 체감 지연 최소화: 부분 결과 선반영 + 릴리즈 후 2초 내 강제 송신.
@Observable
@MainActor
final class VoiceInputManager {
    enum VoiceState: Equatable { case idle, listening, processing }

    private(set) var state: VoiceState = .idle
    private(set) var enabled = false
    /// 권한/환경 문제 설명 — 음성 토글 버튼 툴팁으로 노출
    var statusNote: String?

    /// 인식 완료 문장 전달 (ShellModel.send 연결)
    var onTransmit: ((String) -> Void)?
    /// 청취 시작 즉시 호출 — TTS 재생 중지 연결
    var onListeningStart: (() -> Void)?

    private let recognizer = SFSpeechRecognizer(locale: Locale(identifier: "ko-KR"))
    private let engine = AVAudioEngine()
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var transcript = ""
    private var tapInstalled = false
    private var listenDeadline: Task<Void, Never>?
    private var deliverDeadline: Task<Void, Never>? // 최종 결과 미도착 시 부분 결과 송신 유예
    private var hotKeyRef: EventHotKeyRef?
    private var handlerRef: EventHandlerRef?
    private var prepared = false

    /// 권한 요청 + 핫키 등록. 사용 설명 키가 없는 빌드(예: bare swift run)는 TCC 접근 시
    /// 즉시 크래시하므로 Info.plist 검사로 사전 차단하고 음성을 끈 채로 둔다.
    func prepare() async {
        guard !prepared else { return }
        prepared = true
        guard recognizer != nil else { statusNote = "ko-KR 음성 인식기를 사용할 수 없음"; return }
        let info = Bundle.main.infoDictionary ?? [:]
        guard info["NSMicrophoneUsageDescription"] != nil,
              info["NSSpeechRecognitionUsageDescription"] != nil else {
            statusNote = "마이크/음성인식 사용 설명 없음 — Xcode(app) 빌드에서만 동작"
            return
        }
        let auth = await withCheckedContinuation { cont in
            SFSpeechRecognizer.requestAuthorization { cont.resume(returning: $0) }
        }
        guard auth == .authorized else { statusNote = "음성 인식 권한 거부됨"; return }
        let mic = await withCheckedContinuation { cont in
            AVAudioApplication.requestRecordPermission { cont.resume(returning: $0) }
        }
        guard mic else { statusNote = "마이크 권한 거부됨"; return }
        registerHotkey()
        enabled = true
    }

    /// 음성 토글 — 끄면 핫키 해제·청취 중단, 켜면 권한 절차부터 재수행
    func setEnabled(_ on: Bool) {
        guard enabled != on else { return }
        guard on else {
            unregisterHotkey()
            abort()
            enabled = false
            return
        }
        prepared = false
        Task { await prepare() } // 성공 시 enabled = true
    }

    func hotkeyDown() {
        guard enabled, state == .idle, let recognizer else { return }
        onListeningStart?()
        teardownAudio()
        transcript = ""
        let req = SFSpeechAudioBufferRecognitionRequest()
        req.shouldReportPartialResults = true
        if recognizer.supportsOnDeviceRecognition { req.requiresOnDeviceRecognition = true } // 온디바이스 — 저지연·오프라인
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else { statusNote = "입력 오디오 장치 없음"; return }
        input.installTap(onBus: 0, bufferSize: 4096, format: format) { buffer, _ in req.append(buffer) }
        tapInstalled = true
        do {
            engine.prepare()
            try engine.start()
        } catch {
            statusNote = "오디오 시작 실패: \(error.localizedDescription)"
            teardownAudio()
            return
        }
        request = req
        state = .listening
        listenDeadline = Task { [weak self] in // 키 릴리즈 이벤트 유실 대비 안전판
            try? await Task.sleep(for: .seconds(30))
            guard !Task.isCancelled else { return }
            self?.hotkeyUp()
        }
        task = recognizer.recognitionTask(with: req) { [weak self] result, error in
            let partial = result?.bestTranscription.formattedString // 스냅숏만 건네 Sendable 유지
            let final = result?.isFinal ?? false
            let failed = error != nil
            Task { @MainActor [weak self] in
                self?.handle(partial: partial, isFinal: final, failed: failed)
            }
        }
    }

    func hotkeyUp() {
        guard state == .listening else { return }
        state = .processing
        listenDeadline?.cancel(); listenDeadline = nil
        teardownAudio()
        request?.endAudio()
        deliverDeadline = Task { [weak self] in // 최종 결과 2초 내 미도착 → 부분 결과로 송신
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            self?.finish()
        }
    }

    private func handle(partial: String?, isFinal: Bool, failed: Bool) {
        if let partial { transcript = partial }
        if isFinal || failed { finish() } // 릴리즈 전 확정(정적 타임아웃 등)도 즉시 송신
    }

    private func finish() {
        guard state != .idle else { return }
        deliverDeadline?.cancel(); deliverDeadline = nil
        task?.finish(); task = nil
        request = nil
        teardownAudio()
        let text = transcript.trimmingCharacters(in: .whitespacesAndNewlines)
        transcript = ""
        state = .idle
        if !text.isEmpty { onTransmit?(text) }
    }

    /// 청취/처리 강제 중단 — 결과 전송 없음 (토글 끔 시)
    private func abort() {
        listenDeadline?.cancel(); listenDeadline = nil
        deliverDeadline?.cancel(); deliverDeadline = nil
        task?.cancel(); task = nil
        request = nil
        teardownAudio()
        transcript = ""
        state = .idle
    }

    private func teardownAudio() {
        if engine.isRunning { engine.stop() }
        if tapInstalled { engine.inputNode.removeTap(onBus: 0); tapInstalled = false }
    }

    // MARK: Carbon 핫키 (Cmd+Shift+Space) — 전역 PTT는 접근성 권한 없이도 동작하는 표준 경로

    private func registerHotkey() {
        guard hotKeyRef == nil else { return }
        var specs = [
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed)),
            EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyReleased)),
        ]
        let installed = InstallEventHandler(
            GetApplicationEventTarget(),
            pttHotKeyHandler,
            2,
            &specs,
            Unmanaged.passUnretained(self).toOpaque(),
            &handlerRef
        )
        guard installed == noErr else { statusNote = "핫키 핸들러 등록 실패 (\(installed))"; return }
        var ref: EventHotKeyRef?
        let registered = RegisterEventHotKey(
            UInt32(kVK_Space),
            UInt32(cmdKey | shiftKey),
            EventHotKeyID(signature: 0x4155_5450, id: 1), // 'AUTP'
            GetApplicationEventTarget(),
            0,
            &ref
        )
        guard registered == noErr, let ref else { statusNote = "Cmd+Shift+Space 등록 실패 (\(registered))"; return }
        hotKeyRef = ref
    }

    private func unregisterHotkey() {
        if let hotKeyRef { UnregisterEventHotKey(hotKeyRef); self.hotKeyRef = nil }
        if let handlerRef { RemoveEventHandler(handlerRef); self.handlerRef = nil }
    }

    isolated deinit {
        if let hotKeyRef { UnregisterEventHotKey(hotKeyRef) }
        if let handlerRef { RemoveEventHandler(handlerRef) }
    }
}

/// Carbon 이벤트 핸들러 — 애플리케이션(메인) 이벤트 루프에서 호출되므로 메인 스레드 보장.
private func pttHotKeyHandler(
    _ callRef: EventHandlerCallRef?,
    _ event: EventRef?,
    _ userData: UnsafeMutableRawPointer?
) -> OSStatus {
    guard let event, let userData else { return noErr }
    let kind = GetEventKind(event)
    let manager = Unmanaged<VoiceInputManager>.fromOpaque(userData).takeUnretainedValue()
    MainActor.assumeIsolated {
        if kind == UInt32(kEventHotKeyPressed) { manager.hotkeyDown() }
        else if kind == UInt32(kEventHotKeyReleased) { manager.hotkeyUp() }
    }
    return noErr
}
