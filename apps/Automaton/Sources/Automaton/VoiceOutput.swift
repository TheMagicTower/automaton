import AVFoundation

/// §12 1단계 TTS — StreamDelta 누적 → 문장 단위 한국어 음성 출력. 새 입력 도착 시 재생·큐 즉시 중지.
@MainActor
final class VoiceOutputManager {
    private let synthesizer = AVSpeechSynthesizer()
    private let voice: AVSpeechSynthesisVoice? = VoiceOutputManager.bestKoreanVoice()
    private var buffer = ""
    var isMuted: Bool {
        didSet { UserDefaults.standard.set(isMuted, forKey: "automaton.voiceMuted") }
    }

    init() {
        self.isMuted = UserDefaults.standard.bool(forKey: "automaton.voiceMuted")
    }

    /// 사용 가능한 최고 품질 한국어 음성 선택 — Premium > Enhanced > Compact(Yuna)
    private static func bestKoreanVoice() -> AVSpeechSynthesisVoice? {
        let all = AVSpeechSynthesisVoice.speechVoices().filter { $0.language.hasPrefix("ko") }
        // Premium(3) > Enhanced(2) > Default(1) 순서로 선택, 같은 품질이면 Yuna 선호
        return all.first { $0.quality == .premium }
            ?? all.first { $0.quality == .enhanced }
            ?? all.first { $0.name == "Yuna" }
            ?? all.first
            ?? AVSpeechSynthesisVoice(language: "ko-KR")
    }

    func append(delta: String) {
        guard !delta.isEmpty, !isMuted else { return }

        buffer += delta
        flushCompleteSentences()
        if buffer.count > 160 {
            speak(buffer)
            buffer.removeAll()
        }
    }

    func flush() {
        flushCompleteSentences()
        let rest = buffer.trimmingCharacters(in: .whitespacesAndNewlines)
        buffer.removeAll()
        if !rest.isEmpty { speak(rest) }
    }

    func interrupt() {
        synthesizer.stopSpeaking(at: .immediate)
        buffer.removeAll()
    }

    private func flushCompleteSentences() {
        while let i = buffer.firstIndex(where: { Self.terminators.contains($0) }) {
            let end = buffer.index(after: i)
            let sentence = String(buffer[..<end]).trimmingCharacters(in: .whitespacesAndNewlines)
            buffer.removeSubrange(..<end)
            if !sentence.isEmpty { speak(sentence) }
        }
    }

    private func speak(_ text: String) {
        let u = AVSpeechUtterance(string: text)
        u.voice = voice
        u.rate = 0.48 // 기본값 0.5보다 약간 느리게 — 한국어 자연스러움
        u.pitchMultiplier = 1.0
        u.postUtteranceDelay = 0.15 // 문장 간 자연스러운 쉼
        synthesizer.speak(u)
    }

    private static let terminators: Set<Character> = [".", "!", "?", "…", "。", "！", "？", "\n", ";", "~"]
}
