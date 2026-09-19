import AppKit
import SwiftUI

// MARK: - 다중 행 입력 필드 (Enter=전송 · Shift+Enter=줄바꿈)

/// 한글 IME 조합 중 Enter는 조합 확정으로 넘기고(전송 아님), 확정 상태의 Enter만 전송
final class ComposerTextView: NSTextView {
    var onSend: (() -> Void)?

    override func keyDown(with event: NSEvent) {
        let isReturn = event.keyCode == 36 || event.keyCode == 76 // Return · keypad Enter
        let shift = event.modifierFlags.intersection(.deviceIndependentFlagsMask).contains(.shift)
        if isReturn, !shift, markedRange().length == 0 {
            onSend?()
            return
        }
        super.keyDown(with: event) // Shift+Enter=줄바꿈, 조합 중 Enter=확정
    }

    /// 뷰가 창에 붙는 시점에 입력 포커스 획득 — 구 TextField onAppear 포커스 동작 계승
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window != nil {
            window?.makeFirstResponder(self)
        }
    }
}

/// NSTextView 래퍼 — 자동 높이 성장(38~140) + 1000자 상한
struct MultilineInputField: NSViewRepresentable {
    static let maxLength = 1000

    @Binding var text: String
    @Binding var contentHeight: CGFloat
    var onSend: () -> Void

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: MultilineInputField

        init(_ parent: MultilineInputField) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard let tv = notification.object as? ComposerTextView else { return }
            // 1000자 상한 — 초과분 절단 (대량 붙여넣기 시에만 발생)
            if tv.string.count > MultilineInputField.maxLength {
                tv.string = String(tv.string.prefix(MultilineInputField.maxLength))
            }
            parent.text = tv.string
            parent.contentHeight = Self.measure(tv)
        }

        /// 내용 높이 측정 — 하한 38, 상한 140(넘으면 스크롤)
        static func measure(_ tv: NSTextView) -> CGFloat {
            guard let lm = tv.layoutManager, let container = tv.textContainer else { return 38 }
            let used = lm.usedRect(for: container)
            return min(max(used.height + tv.textContainerInset.height * 2 + 4, 38), 140)
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.hasHorizontalScroller = false
        scroll.borderType = .noBorder
        scroll.drawsBackground = false

        let tv = ComposerTextView()
        tv.autoresizingMask = [.width]
        tv.isVerticallyResizable = true
        tv.isHorizontallyResizable = false
        tv.textContainer?.widthTracksTextView = true
        tv.textContainer?.containerSize = NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude)
        tv.textContainerInset = NSSize(width: 0, height: 6)
        tv.textContainer?.lineFragmentPadding = 0
        tv.font = NSFont.systemFont(ofSize: 13)
        tv.textColor = NSColor(red: 0.85, green: 0.80, blue: 0.70, alpha: 1) // Theme.ivory
        tv.backgroundColor = .clear
        tv.drawsBackground = false
        tv.allowsUndo = true
        tv.isRichText = false
        tv.string = text
        tv.onSend = onSend
        tv.delegate = context.coordinator

        scroll.documentView = tv
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.parent = self
        guard let tv = scroll.documentView as? ComposerTextView else { return }
        tv.onSend = onSend
        if tv.string != text {
            tv.string = text
            context.coordinator.parent.contentHeight = Coordinator.measure(tv)
        }
    }
}

// MARK: - 입력 바 (문자 수 카운터 + 전송 버튼)

/// 개선된 입력창 — 다중 행, 1000자 카운터, 전송 버튼(isThinking 중 비활성 + 로딩 애니메이션)
struct ComposerBar: View {
    @Environment(ShellModel.self) private var model
    @Binding var draft: String
    @State private var inputHeight: CGFloat = 38

    private var canSend: Bool {
        !model.isThinking && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        VStack(spacing: 4) {
            HStack(alignment: .bottom, spacing: 8) {
                MultilineInputField(text: $draft, contentHeight: $inputHeight, onSend: send)
                    .frame(height: inputHeight)
                    .background(RoundedRectangle(cornerRadius: 8).fill(Theme.walnutPanel.opacity(0.6)))
                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Theme.brass.opacity(0.35)))
                sendButton
            }
            hintRow
        }
        .padding(.horizontal, 10)
        .padding(.top, 8)
        .padding(.bottom, 6)
    }

    private func send() {
        guard canSend else { return }
        let text = draft
        draft = ""
        model.send(text)
    }

    private var sendButton: some View {
        Button(action: send) {
            Group {
                if model.isThinking {
                    // 로딩 애니메이션 — 본체 톱니 인디케이터와 동일 리듬(2s/회전)
                    TimelineView(.animation(minimumInterval: 1.0 / 30.0)) { timeline in
                        let angle = (timeline.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 2.0)) / 2.0 * 360.0
                        Image(systemName: "gearshape.fill")
                            .font(.system(size: 14, weight: .medium))
                            .foregroundStyle(Theme.brass)
                            .rotationEffect(.degrees(angle))
                    }
                } else {
                    Image(systemName: "paperplane.fill")
                        .font(.system(size: 14))
                        .foregroundStyle(canSend ? Theme.gold : Theme.dim.opacity(0.5))
                }
            }
            .frame(width: 30, height: 30)
        }
        .buttonStyle(.plain)
        .disabled(!canSend)
        .help(model.isThinking ? "응답 대기 중…" : "전송 (Enter)")
    }

    private var hintRow: some View {
        HStack {
            Text("⏎ 전송 · ⇧⏎ 줄바꿈")
                .font(.system(size: 9))
                .foregroundStyle(Theme.dim.opacity(0.7))
            Spacer()
            Text("\(draft.count) / \(MultilineInputField.maxLength)")
                .font(.system(size: 9, design: .monospaced))
                .monospacedDigit()
                .foregroundStyle(countColor)
        }
    }

    private var countColor: Color {
        if draft.count >= MultilineInputField.maxLength { return ChatPalette.error }
        if draft.count >= MultilineInputField.maxLength - 100 { return Theme.gold }
        return Theme.dim.opacity(0.8)
    }
}
