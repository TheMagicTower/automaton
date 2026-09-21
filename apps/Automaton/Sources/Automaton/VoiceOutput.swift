import AVFoundation

/// §12 1단계 TTS — StreamDelta 누적 → 문장 단위 한국어 음성 출력. 새 입력 도착 시 재생·큐 즉시 중지.
@MainActor
final class VoiceOutputManager {
    private let synthesizer = AVSpeechSynthesizer()
    private let voice = AVSpeechSynthesisVoice(language: "ko-KR")
    private var buffer = ""
    /// 음성 출력 켜기/끄기 — 기본값 UserDefaults에서 복원
    var isMuted: Bool {
        didSet { UserDefaults.standard.set(isMuted, forKey: "automaton.voiceMuted") }
    }

    init() {
        self.isMuted = UserDefaults.standard.bool(forKey: "automaton.voiceMuted")
    }

    /// 스트림 델타 누적 — 종결 부호 도달 문장은 즉시 발화 큐에 삽입
    func append(delta: String) {
        guard !delta.isEmpty, !isMuted else { return }

        buffer += delta
        flushCompleteSentences()
        if buffer.count > 160 { // 종결 부표 없는 장문 누적 → 청크 출력 (첫 발화 지연 방지)
            speak(buffer)
            buffer.removeAll()
        }
    }

    /// 스트림 끝(도구 시작 등) — 잔여 버퍼 마저 발화
    func flush() {
        flushCompleteSentences()
        let rest = buffer.trimmingCharacters(in: .whitespacesAndNewlines)
        buffer.removeAll()
        if !rest.isEmpty { speak(rest) }
    }

    /// 새 입력 — 재생 중 발화·대기 큐 즉시 중지, 미발화 버퍼 폐기
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
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = voice
        synthesizer.speak(utterance) // 이미 발화 중이면 순차 큐잉
    }

    private static let terminators: Set<Character> = [".", "!", "?", "…", "。", "！", "？", "\n", ";", "~"]
}
