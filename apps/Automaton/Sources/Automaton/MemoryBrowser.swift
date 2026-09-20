import SwiftUI

/// 기억 브라우저 (§7) — memory_browse/memory_stats/memory_delete RPC로
/// 데몬에 저장된 사용자 facts를 조회·삭제하는 팝오버. 우클릭 메뉴 "🧠 기억 브라우저"에서 표시.
/// Brass & Glass — 월넛 배경 + 황동 액센트.
struct MemoryBrowser: View {
    @Environment(ShellModel.self) private var model

    /// 한 번에 불러올 페이지 크기 — M1 개인 기억 규모 감안 여유 있게
    private static let pageSize = 100

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().overlay(Theme.brass.opacity(0.4))
            factsList
            Divider().overlay(Theme.brass.opacity(0.4))
            footer
        }
        .frame(minWidth: 360, minHeight: 420)
        .background(Theme.walnut)
        .onAppear { model.browseMemory() }
    }

    private var header: some View {
        HStack {
            Theme.title("🧠 기억 브라우저")
            Spacer()
            Button {
                model.browseMemory()
            } label: {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Theme.gold)
            }
            .buttonStyle(.plain)
            .help("다시 불러오기")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
    }

    /// facts 목록 — 월넛 패널 카드 + 개별 삭제 버튼
    private var factsList: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 6) {
                if model.memoryFacts.isEmpty { emptyState }
                ForEach(model.memoryFacts, id: \.self) { fact in
                    MemoryFactRow(fact: fact) { model.deleteMemory(fact) }
                }
            }
            .padding(10)
        }
    }

    /// 저장된 기억이 없을 때 안내
    private var emptyState: some View {
        VStack(spacing: 10) {
            Image(systemName: "brain")
                .font(.system(size: 26))
                .foregroundStyle(Theme.brass.opacity(0.6))
            Text(model.connected ? "저장된 기억이 없습니다" : "데몬에 연결할 수 없습니다")
                .font(.system(size: 12, design: .serif))
                .foregroundStyle(Theme.dim)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 56)
        .padding(.bottom, 24)
    }

    /// 하단 통계 — memory_stats 응답(총 fact 수·세션 수·결정 수)
    private var footer: some View {
        HStack(spacing: 8) {
            if let s = model.memoryStats {
                Text("🧠 \(s.facts)건 · 세션 \(s.sessions)개 · 결정 \(s.decisions)건")
            } else if model.memoryTotal > 0 {
                Text("🧠 \(model.memoryTotal)건")
            } else {
                Text(" ")
            }
            Spacer()
            // 페이지 밖 남은 fact 알림 — M1 규모에선 거의 표시되지 않음
            if model.memoryTotal > Self.pageSize {
                Text("처음 \(Self.pageSize)건 표시")
            }
        }
        .font(.system(size: 10, design: .monospaced))
        .foregroundStyle(Theme.dim)
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }
}

/// 개별 fact 행 — 아이보리 본문 + 휴지통 버튼(memory_delete)
private struct MemoryFactRow: View {
    let fact: String
    let onDelete: () -> Void
    @State private var hovering = false

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Text(fact)
                .font(.system(size: 11))
                .foregroundStyle(Theme.ivory)
                .fixedSize(horizontal: false, vertical: true) // 긴 fact 줄바꿈 보장
            Spacer(minLength: 4)
            Button(action: onDelete) {
                Image(systemName: "trash")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(hovering ? ChatPalette.error : Theme.dim)
            }
            .buttonStyle(.plain)
            .help("이 기억 삭제")
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(RoundedRectangle(cornerRadius: 6).fill(Theme.walnutPanel))
        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Theme.brass.opacity(0.35)))
        .onHover { hovering = $0 }
    }
}
