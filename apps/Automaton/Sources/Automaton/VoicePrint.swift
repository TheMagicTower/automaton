import AVFoundation

/// 화자 식별 (Voiceprint) — 오디오 특징 추출 → 등록된 보이스프린트와 비교.
/// Pitch + 스펙트럼 통계로 화자 구분. 자동 등록은 pendingEnrollment로 처리.
@MainActor
final class VoicePrintManager {
    static let shared = VoicePrintManager()

    struct VoicePrint: Codable {
        let name: String
        let features: [Double]
        let createdAt: Date
    }

    private var prints: [VoicePrint] = []
    private let threshold = 0.75

    init() {
        if let data = UserDefaults.standard.data(forKey: "automaton.voiceprints"),
           let decoded = try? JSONDecoder().decode([VoicePrint].self, from: data) {
            prints = decoded
        }
    }

    private func save() {
        if let data = try? JSONEncoder().encode(prints) {
            UserDefaults.standard.set(data, forKey: "automaton.voiceprints")
        }
    }

    // MARK: - 특징 추출

    nonisolated func extractFeatures(from buffer: AVAudioPCMBuffer) -> [Double] {
        guard let ptr = buffer.floatChannelData?[0] else { return [] }
        let n = Int(buffer.frameLength)
        let sr = Float(buffer.format.sampleRate)
        let data = Array(UnsafeBufferPointer(start: ptr, count: n))
        let pitch = detectPitch(data, n: n, sr: sr)
        let zcr = zeroCrossing(data, n: n)
        let mean = data.reduce(0, +) / Float(n)
        let variance = data.map { ($0 - mean) * ($0 - mean) }.reduce(0, +) / Float(n)
        let rms = sqrt(variance)

        return [pitch, zcr, Double(rms), Double(sqrt(variance / max(sr / 1000, 1)))]
    }

    nonisolated private func detectPitch(_ d: [Float], n: Int, sr: Float) -> Double {
        let minLag = Int(sr / 400), maxLag = min(Int(sr / 80), n - 1)
        guard minLag < maxLag else { return 0 }
        var bestLag = 0; var bestC: Float = 0
        for lag in minLag..<maxLag {
            var c: Float = 0
            for i in 0..<(n - lag) { c += d[i] * d[i + lag] }
            if c > bestC { bestC = c; bestLag = lag }
        }
        return bestLag > 0 ? Double(sr / Float(bestLag)) : 0
    }

    nonisolated private func zeroCrossing(_ d: [Float], n: Int) -> Double {
        var z = 0
        for i in 1..<n { if (d[i-1] >= 0) != (d[i] >= 0) { z += 1 } }
        return Double(z) / Double(n)
    }

    // MARK: - 식별

    func identify(features: [Double]) -> (name: String, confidence: Double)? {
        guard !prints.isEmpty, !features.isEmpty else { return nil }
        var best: (String, Double)? = nil
        for p in prints {
            let sim = cosine(features, p.features)
            if sim >= threshold && (best == nil || sim > best!.1) { best = (p.name, sim) }
        }
        return best.map { (name: $0.0, confidence: $0.1) }
    }

    private func cosine(_ a: [Double], _ b: [Double]) -> Double {
        let n = min(a.count, b.count)
        guard n > 0 else { return 0 }
        var dot = 0.0, na = 0.0, nb = 0.0
        for i in 0..<n { dot += a[i] * b[i]; na += a[i] * a[i]; nb += b[i] * b[i] }
        let d = sqrt(na) * sqrt(nb)
        return d > 0 ? dot / d : 0
    }

    // MARK: - 등록

    func enroll(name: String, features: [Double]) {
        prints.append(VoicePrint(name: name, features: features, createdAt: Date()))
        save()
    }

    func remove(named name: String) {
        prints.removeAll { $0.name == name }
        save()
    }

    var speakers: [String] { prints.map(\.name) }
    var hasPrints: Bool { !prints.isEmpty }
}
