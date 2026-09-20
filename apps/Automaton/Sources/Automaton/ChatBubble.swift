import SwiftUI

// MARK: - 대화 데이터 모델

/// 대화 참여자 역할 — 버블 스타일 결정
enum UserRole: String, Codable {
    case user   // 사용자 — 우측 황동 버블
    case agent  // 에이전트 — 좌측 월넛 패널 버블
    case tool   // 툴 실행 — ⚙/✓/✗ 모노스페이스 라인
    case error  // 시스템 오류 — 붉은 텍스트
}

/// 구조화된 대화 엔트리 — 구버전 stream[String] 텍스트 blob을 대체
struct ChatEntry: Identifiable, Codable {
    let id: UUID
    let role: UserRole
    var content: String
    let timestamp: Date

    init(id: UUID = UUID(), role: UserRole, content: String, timestamp: Date = Date()) {
        self.id = id
        self.role = role
        self.content = content
        self.timestamp = timestamp
    }
}

extension ChatEntry {
    /// 구버전 stream[String] 텍스트 blob 호환 변환.
    /// 데몬 history_get이 "▸ " 접두 사용자 메시지 + 에이전트 텍스트를 개행 조인
    /// 단일 blob으로 흘려보내므로, 라인 접두로 role을 복원한다.
    /// "▸"=user · "⚙/✓/✗"=tool · "⚠"=error · 나머지=agent(연속 라인은 한 버블로 병합).
    static func parseLegacy(_ blob: String) -> [ChatEntry] {
        var out: [ChatEntry] = []
        for raw in blob.split(separator: "\n", omittingEmptySubsequences: true) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            guard !line.isEmpty else { continue }
            let entry: ChatEntry
            if line.hasPrefix("▸ ") {
                entry = ChatEntry(role: .user, content: String(line.dropFirst(2)))
            } else if line.hasPrefix("▸") {
                entry = ChatEntry(role: .user, content: String(line.dropFirst()))
            } else if line.hasPrefix("⚙") || line.hasPrefix("✓") || line.hasPrefix("✗") {
                entry = ChatEntry(role: .tool, content: line)
            } else if line.hasPrefix("⚠") {
                entry = ChatEntry(role: .error, content: String(line.dropFirst()).trimmingCharacters(in: .whitespaces))
            } else {
                entry = ChatEntry(role: .agent, content: line)
            }
            // 연속 agent 라인(멀티라인 응답)만 한 버블로 병합 — 툴/사용자는 개별 유지
            if let last = out.last, last.role == .agent, entry.role == .agent {
                out[out.count - 1].content += "\n" + entry.content
            } else {
                out.append(entry)
            }
        }
        return out
    }
}

/// 상태 색 — 테마에 없는 성공(녹)/오류(적) 톤, Brass & Glass 조화 채도로 로컬 정의
enum ChatPalette {
    static let error = Color(red: 0.82, green: 0.40, blue: 0.34)
    static let ok = Color(red: 0.55, green: 0.72, blue: 0.48)
}

// MARK: - 버블 뷰

/// 역할별 대화 버블 — 사용자=우측 황동, 에이전트=좌측 월넛 패널, 툴/오류=라인형
struct ChatBubble: View {
    let entry: ChatEntry
    @State private var expanded = false
    var body: some View {
        switch entry.role {
        case .user: userBubble
        case .agent: agentBubble
        case .tool: toolLine
        case .error: errorLine
        }
    }

    /// 사용자 — 우측 정렬, 황동 그라디언트 (월넛 글자 = 대비 확보)
    private var userBubble: some View {
        HStack {
            Spacer(minLength: 56)
            Text(entry.content)
                .font(.system(size: 13))
                .foregroundStyle(Theme.walnut)
                .multilineTextAlignment(.leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(
                    RoundedRectangle(cornerRadius: 12)
                        .fill(LinearGradient(colors: [Theme.gold.opacity(0.92), Theme.brass], startPoint: .top, endPoint: .bottom))
                )
                .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Theme.gold.opacity(0.55)))
                .textSelection(.enabled)
        }
    }

    /// 에이전트 — 좌측 정렬, 월넛 패널
    private var agentBubble: some View {
        HStack {
            Text(entry.content)
                .font(.system(size: 13))
                .foregroundStyle(Theme.ivory)
                .multilineTextAlignment(.leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(RoundedRectangle(cornerRadius: 12).fill(Theme.walnutPanel))
                .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Theme.brass.opacity(0.25)))
                .textSelection(.enabled)
            Spacer(minLength: 56)
        }
    }

    /// 툴 실행 — 접이식: 기본 2줄, 클릭하면 전체 출력 펼침
    private var toolLine: some View {
        let isLong = entry.content.count > 80
        return HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(String(entry.content.prefix(1)))
                .font(.system(size: 11))
                .foregroundStyle(toolStatusColor)
            Text(String(entry.content.dropFirst()).trimmingCharacters(in: .whitespaces))
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(Theme.dim)
                .lineLimit(expanded ? nil : 2)
                .truncationMode(.tail)
                .textSelection(.enabled)
            if isLong {
                Image(systemName: expanded ? "chevron.up" : "chevron.down")
                    .font(.system(size: 9))
                    .foregroundStyle(Theme.brass.opacity(0.6))
            }
            Spacer(minLength: 0)
        }
        .padding(.leading, 4)
        .contentShape(Rectangle())
        .onTapGesture { if isLong { expanded.toggle() } }
    }

    private var toolStatusColor: Color {
        if entry.content.hasPrefix("✓") { return ChatPalette.ok }
        if entry.content.hasPrefix("✗") { return ChatPalette.error }
        return Theme.brass // ⚙ 실행 중
    }

    /// 시스템 오류 — 붉은 텍스트
    private var errorLine: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 10))
                .foregroundStyle(ChatPalette.error)
            Text(entry.content)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(ChatPalette.error)
                .textSelection(.enabled)
            Spacer(minLength: 0)
        }
        .padding(.leading, 4)
    }
}
