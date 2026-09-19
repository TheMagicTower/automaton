import SwiftUI

/// Brass & Glass 팔레트 — 다크 월넛 + 황동 + 세리프 제목. M1은 코드 상수, 테마 파일화는 후속.
enum Theme {
    static let walnut = Color(red: 0.10, green: 0.09, blue: 0.07)
    static let walnutPanel = Color(red: 0.14, green: 0.12, blue: 0.08)
    static let brass = Color(red: 0.69, green: 0.55, blue: 0.34)
    static let gold = Color(red: 0.79, green: 0.64, blue: 0.15)
    static let ivory = Color(red: 0.85, green: 0.80, blue: 0.70)
    static let dim = Color(red: 0.54, green: 0.48, blue: 0.36)

    static func title(_ s: String) -> some View {
        Text(s).font(.system(.title3, design: .serif)).foregroundStyle(gold)
    }
}
