import AppKit
import SwiftUI

/// AppKit 기반 메뉴바 컨트롤러 — NSStatusItem + NSPopover
/// 프로덕션 메뉴바 앱 표준 방식 + 종료/재기동/새 세션 지원
@MainActor
final class MenuBarController: NSObject, NSPopoverDelegate, NSWindowDelegate {
    private var statusItem: NSStatusItem!
    private var popover: NSPopover!
    private var model: ShellModel!
    private var voiceInput: VoiceInputManager!
    private var eventMonitor: Any?
    private var mainWindow: NSWindow?
    /// 기억 브라우저 팝오버 — 우클릭 메뉴에서 표시 (§7), 바깥 클릭으로 닫힘
    private var memoryPopover: NSPopover?

    func setup(model: ShellModel, voiceInput: VoiceInputManager) {
        self.model = model
        self.voiceInput = voiceInput

        // 기존 인스턴스 종료 (재기동 지원)
        terminateExistingInstances()

        // NSStatusItem 생성
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        if let button = statusItem.button {
            if let img = NSImage(systemSymbolName: "gearshape.fill", accessibilityDescription: "automaton") {
                button.image = img
            } else {
                button.title = "⚙" // SF Symbol 로드 실패 시 폴백
            }
            button.action = #selector(statusItemClicked)
            button.target = self
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
        }
        FileHandle.standardError.write(Data("[automaton] statusItem created: \(statusItem != nil)\n".utf8))

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

        let windowItem = NSMenuItem(title: "창 모드 (⌘Tab)", action: #selector(showWindowAction), keyEquivalent: "w")
        windowItem.target = self
        menu.addItem(windowItem)

        let newSessionItem = NSMenuItem(title: "새 세션", action: #selector(newSessionAction), keyEquivalent: "n")
        newSessionItem.target = self
        menu.addItem(newSessionItem)

        let memoryItem = NSMenuItem(title: "🧠 기억 브라우저", action: #selector(showMemoryBrowserAction), keyEquivalent: "m")
        memoryItem.target = self
        menu.addItem(memoryItem)

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

    // MARK: - 창 모드 (⌘Tab 전환 지원)

    @objc private func showWindowAction() {
        if let window = mainWindow {
            window.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }


        // 정규 창 생성 — .regular 정책으로 Cmd+Tab에 표시.
        // 사이드바(150pt) 폭을 감안해 팝오버보다 넓게 시작.
        NSApp.setActivationPolicy(.regular)

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 700),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "automaton"
        window.center()
        window.delegate = self

        let hostingView = NSHostingView(
            rootView: ShellView(showSidebar: true) // 창 모드 — 세션 사이드바 표시
                .environment(model)
                .environment(voiceInput)
        )
        window.contentView = hostingView

        // Brass & Glass 배경
        window.backgroundColor = NSColor(red: 0.10, green: 0.09, blue: 0.07, alpha: 1)
        window.titlebarAppearsTransparent = true
        window.isMovableByWindowBackground = true

        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        mainWindow = window
    }

    /// 창이 닫히면 메뉴바 전용 모드로 복귀 (Dock·Cmd+Tab에서 제거)
    func windowWillClose(_ notification: Notification) {
        mainWindow = nil
        NSApp.setActivationPolicy(.accessory)
    }

    @objc private func newSessionAction() {
        model.newSession()
        closePopover()
        showPopover()
    }

    /// 기억 브라우저 팝오버 표시 — 메인 팝오버를 닫고 상태 아이콘에 붙여 띄운다.
    /// .transient 동작이라 별도 이벤트 모니터 없이 바깥 클릭으로 닫힌다.
    @objc private func showMemoryBrowserAction() {
        guard let button = statusItem.button else { return }
        if popover.isShown { closePopover() }
        if memoryPopover == nil {
            let pop = NSPopover()
            pop.contentSize = NSSize(width: 380, height: 460)
            pop.behavior = .transient
            pop.animates = true
            let hosting = NSHostingView(rootView: MemoryBrowser().environment(model))
            let vc = NSViewController()
            vc.view = hosting
            pop.contentViewController = vc
            memoryPopover = pop
        }
        memoryPopover?.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
    }

    @objc private func quitAction() {
        // 음성 입력 정리
        voiceInput.setEnabled(false)

        // 세션 대화 즉시 저장 — 디바운스 대기분 유실 방지
        model.flushSessions()

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
