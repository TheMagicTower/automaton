import AppKit
import SwiftUI

/// AppKit 기반 메뉴바 컨트롤러 — NSStatusItem + NSPopover
/// 프로덕션 메뉴바 앱 표준 방식 + 종료/재기동/새 세션 지원
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

        // 기존 인스턴스 종료 (재기동 지원)
        terminateExistingInstances()

        // NSStatusItem 생성
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "gearshape.fill", accessibilityDescription: "automaton")
            button.action = #selector(statusItemClicked)
            button.target = self
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
        }

        // NSPopover — 내부 클릭 시 닫히지 않음
        popover = NSPopover()
        popover.contentSize = NSSize(width: 400, height: 520)
        popover.behavior = .applicationDefined
        popover.animates = true
        popover.delegate = self

        // SwiftUI 뷰 호스팅
        let hostingView = NSHostingView(
            rootView: ShellView()
                .environment(model)
                .environment(voiceInput)
        )
        let controller = NSViewController()
        controller.view = hostingView
        popover.contentViewController = controller
    }

    /// 기존 automaton 프로세스 종료 — 재기동 가능하게
    private func terminateExistingInstances() {
        let currentPid = ProcessInfo.processInfo.processIdentifier
        let apps = NSRunningApplication.runningApplications(withBundleIdentifier: "com.themagictower.automaton")
        for app in apps where app.processIdentifier != currentPid {
            app.terminate()
        }
        // 종료 대기
        Thread.sleep(forTimeInterval: 0.3)
    }

    /// 좌클릭: 팝오버 토글 / 우클릭: 컨텍스트 메뉴 (새 세션·종료)
    @objc private func statusItemClicked(_ sender: NSStatusBarButton) {
        let event = NSApp.currentEvent
        if let event = event, event.type == .rightMouseUp {
            showContextMenu(sender)
        } else {
            togglePopover()
        }
    }

    private func showContextMenu(_ sender: NSStatusBarButton) {
        let menu = NSMenu()

        let newSessionItem = NSMenuItem(title: "새 세션", action: #selector(newSessionAction), keyEquivalent: "n")
        newSessionItem.target = self
        menu.addItem(newSessionItem)

        menu.addItem(.separator())

        let quitItem = NSMenuItem(title: "automaton 종료", action: #selector(quitAction), keyEquivalent: "q")
        quitItem.target = self
        menu.addItem(quitItem)

        statusItem.menu = menu
        statusItem.button?.performClick(nil)
        // 메뉴 표시 후 statusItem.menu 초기화 (다음 좌클릭에서 팝오버 열리게)
        DispatchQueue.main.async { [weak self] in
            self?.statusItem.menu = nil
        }
    }

    @objc private func newSessionAction() {
        model.newSession()
        closePopover()
        showPopover()
    }

    @objc private func quitAction() {
        // 음성 입력 정리
        voiceInput.setEnabled(false)

        // 상태 아이템 제거
        if let item = statusItem {
            NSStatusBar.system.removeStatusItem(item)
        }

        // 앱 종료
        NSApp.terminate(nil)
    }

    // MARK: - Popover 관리

    private func togglePopover() {
        if popover.isShown {
            closePopover()
        } else {
            showPopover()
        }
    }

    private func showPopover() {
        guard let button = statusItem.button else { return }
        popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)

        // 키보드 포커스
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [weak self] in
            guard let self = self else { return }
            if let window = self.popover.contentViewController?.view.window {
                window.makeKey()
            }
        }

        // 외부 클릭 시에만 닫기
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
