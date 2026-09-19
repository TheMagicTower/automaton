import AppKit
import SwiftUI

/// AppKit 기반 메뉴바 컨트롤러 — NSStatusItem + NSPopover
/// MenuBarExtra의 알려진 결함(키보드 무반응, 클릭 시 자동 닫힘)을 해결하는
/// 프로덕션 메뉴바 앱(Slack, Discord 등)의 표준 방식
@MainActor
final class MenuBarController: NSObject, NSPopoverDelegate {
    private var statusItem: NSStatusItem!
    private var popover: NSPopover!
    private var model: ShellModel!
    private var voiceInput: VoiceInputManager!
    private var eventMonitor: Any?

    func setup(model: ShellModel, voiceInput: VoiceInputManager) {
        self.model = model
        self.voiceInput = voiceInput

        // NSStatusItem 생성 (메뉴바 아이콘)
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "gearshape.fill", accessibilityDescription: "automaton")
            button.action = #selector(togglePopover)
            button.target = self
        }

        // NSPopover — .applicationDefined로 내부 클릭 시 닫히지 않게
        popover = NSPopover()
        popover.contentSize = NSSize(width: 400, height: 520)
        popover.behavior = .applicationDefined
        popover.animates = true
        popover.delegate = self

        // SwiftUI 뷰를 NSHostingView로 호스팅
        let hostingView = NSHostingView(
            rootView: ShellView()
                .environment(model)
                .environment(voiceInput)
        )
        let controller = NSViewController()
        controller.view = hostingView
        popover.contentViewController = controller
    }

    @objc private func togglePopover() {
        if popover.isShown {
            closePopover()
        } else {
            showPopover()
        }
    }

    private func showPopover() {
        guard let button = statusItem.button else { return }
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)

        // 핵심: 팝오버 창을 key window로 만들어 키보드 입력 활성화
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self] in
            guard let self = self else { return }
            if let window = self.popover.contentViewController?.view.window {
                window.makeKey()
                // TextField에 초기 포커스 부여
                window.firstResponder?.becomeFirstResponder()
            }
        }

        // 전역 마우스 모니터링 — 팝오버 외부 클릭 시에만 닫기
        eventMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.closePopover()
            }
        }
    }

    private func closePopover() {
        popover.performClose(nil)
        if let monitor = eventMonitor {
            NSEvent.removeMonitor(monitor)
            eventMonitor = nil
        }
    }

    // MARK: - NSPopoverDelegate

    func popoverDidClose(_ notification: Notification) {
        if let monitor = eventMonitor {
            NSEvent.removeMonitor(monitor)
            eventMonitor = nil
        }
    }
}
