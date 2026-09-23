import AVFoundation

/// §12 1단계 TTS — StreamDelta 누적 → 문장 단위 한국어 음성 출력. 새 입력 도착 시 재생·큐 즉시 중지.
@MainActor
final class VoiceOutputManager {
    private let synthesizer = AVSpeechSynthesizer()
    private let voice: AVSpeechSynthesisVoice? = VoiceOutputManager.bestKoreanVoice()
    private var buffer = ""
    var isMuted: Bool {
        didSet {
            UserDefaults.standard.set(isMuted, forKey: "automaton.voiceMuted")
            if !isMuted {
                // 뮤트 해제 시 신디사이저 리셋 — 중단된 큐 정리
                synthesizer.stopSpeaking(at: .immediate)
            }
        }
    }
    init() {
        self.isMuted = UserDefaults.standard.bool(forKey: "automaton.voiceMuted")
    }

    /// Premium 음성 식별자로 직접 지정 — 품질 검색보다 확실
    private static func bestKoreanVoice() -> AVSpeechSynthesisVoice? {
        // 1순위: 식별자 직접 지정 (가장 확실)
        if let premium = AVSpeechSynthesisVoice(identifier: "com.apple.voice.premium.ko-KR.Yuna") {
            return premium
        }
        // 2순위: 품질 기반 선택
        let all = AVSpeechSynthesisVoice.speechVoices().filter { $0.language.hasPrefix("ko") }
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
        guard !isMuted else { return }
        let u = AVSpeechUtterance(string: text)
        u.voice = voice
        u.rate = 0.48
        u.pitchMultiplier = 1.0
        u.postUtteranceDelay = 0.15
        u.volume = 1.0 // 명시적 볼륨
        synthesizer.speak(u)
        FileHandle.standardError.write(Data("[automaton] TTS: \"\(text.prefix(30))\" voice=\(voice?.name ?? "nil")\n".utf8))
    }

    private static let terminators: Set<Character> = [".", "!", "?", "…", "。", "！", "？", "\n", ";", "~"]
}
