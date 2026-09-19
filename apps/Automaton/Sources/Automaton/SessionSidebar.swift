import SwiftUI

// MARK: - 세션 기록 모델

/// 사이드바용 세션 기록 — 대화 캐시 + 갱신 시각 (UserDefaults 영속화 대상)
struct SessionRecord: Identifiable, Codable {
    let id: String
    var entries: [ChatEntry]
    var updatedAt: Date
}

extension SessionRecord {
    /// 세션 제목 — 첫 사용자 메시지 (없으면 "새 대화")
    var title: String {
        let first = entries.first(where: { $0.role == .user })?.content ?? ""
        let flat = first.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
        return flat.isEmpty ? "새 대화" : flat
    }

    /// 마지막 메시지 미리보기 — 단일 행
    var preview: String {
        guard let last = entries.last else { return "빈 세션" }
        return last.content.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
    }

    /// 타임스탬프 — 오늘은 시:분, 과거는 월/일
    var timeLabel: String {
        if Calendar.current.isDateInToday(updatedAt) {
            return updatedAt.formatted(.dateTime.hour().minute())
        }
        return updatedAt.formatted(.dateTime.month(.twoDigits).day(.twoDigits))
    }
}

/// UserDefaults 세션 저장소 — Codable 기록 배열 저장/복원
enum SessionStore {
    private static let key = "automaton.sessions"

    static func load() -> [SessionRecord] {
        guard let data = UserDefaults.standard.data(forKey: key) else { return [] }
        return (try? JSONDecoder().decode([SessionRecord].self, from: data)) ?? []
    }

    static func save(_ records: [SessionRecord]) {
        guard let data = try? JSONEncoder().encode(records) else { return }
        UserDefaults.standard.set(data, forKey: key)
    }
}

// MARK: - 사이드바 뷰

/// 창 모드 좌측 세션 목록(150pt) — 최근 순, 현재 세션 황동 보더 하이라이트
struct SessionSidebar: View {
    @Environment(ShellModel.self) private var model

    /// 본체보다 살짝 어두운 월넛 — 채팅 영역과의 시각적 구분
    private static let sidebarBg = Color(red: 0.07, green: 0.06, blue: 0.05)

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().overlay(Theme.brass.opacity(0.4))
            ScrollView {
                LazyVStack(spacing: 4) {
                    ForEach(model.sessions) { record in
                        Button {
                            model.switchSession(record.id)
                        } label: {
                            SessionRow(record: record, isCurrent: record.id == model.currentSessionID)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(6)
            }
        }
        .frame(width: 150)
        .background(Self.sidebarBg)
    }

    private var header: some View {
        HStack {
            Theme.title("세션")
            Spacer()
            Button {
                model.newSession()
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Theme.gold)
            }
            .buttonStyle(.plain)
            .help("새 세션")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
    }
}

private struct SessionRow: View {
    let record: SessionRecord
    let isCurrent: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(record.title)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(isCurrent ? Theme.gold : Theme.ivory)
                .lineLimit(1)
            Text(record.preview)
                .font(.system(size: 10))
                .foregroundStyle(Theme.dim)
                .lineLimit(1)
            Text(record.timeLabel)
                .font(.system(size: 9, design: .monospaced))
                .foregroundStyle(Theme.dim.opacity(0.8))
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 6).fill(isCurrent ? Theme.walnutPanel : Color.clear))
        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(isCurrent ? Theme.brass : Color.clear))
        .contentShape(RoundedRectangle(cornerRadius: 6))
    }
}
