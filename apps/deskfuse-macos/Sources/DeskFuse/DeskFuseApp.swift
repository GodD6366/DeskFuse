import SwiftUI
import AppKit
import Darwin
import DeskFuseEngine

@MainActor struct DeskFuseApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var store = AppStore.shared
    var body: some SwiftUI.Scene {
        Window("", id: "main") {
            RootView(store: store)
                .frame(minWidth: 960, maxWidth: .infinity, minHeight: 568, maxHeight: .infinity)
                .background(WindowIdentity())
        }
        .defaultSize(width: 1200, height: 780)
        // hiddenTitleBar 与 WindowChromeMarker 的手动配置等价（透明标题栏、
        // 隐藏标题、无分隔线），但由 SwiftUI 自己持有：场景因 store 发布而重建时
        // 它会重新应用该配置，不会把 titlebarAppearsTransparent 覆盖回 NO。
        // 用 .titleBar 时重建会把标题栏材质恢复，窗口 resign key 的瞬间
        // 标题栏背景以灰色横条盖在伸入标题栏区的内容上（键鼠共享页最明显）。
        .windowStyle(.hiddenTitleBar)
        .commands {
            CommandGroup(replacing: .newItem) { }
            ClipboardCommands()
            CommandGroup(replacing: .appSettings) {
                Button("设置…") { store.openSettings() }
                    .keyboardShortcut(",", modifiers: .command)
            }
            #if DEBUG
            CommandGroup(after: .windowSize) {
                Button("标准设计窗口（1200 × 780）") { resizeForDesign(width: 1200, height: 780) }
                Button("紧凑设计窗口（960 × 620）") { resizeForDesign(width: 960, height: 620) }
                Divider()
                Button("设计外观：浅色") { NSApp.appearance = NSAppearance(named: .aqua) }
                Button("设计外观：深色") { NSApp.appearance = NSAppearance(named: .darkAqua) }
                Button("设计外观：跟随系统") { NSApp.appearance = nil }
                Divider()
                Button("检查剪贴板面板") {
                    NSApp.windows.first(where: { $0.identifier?.rawValue == "deskfuse-main" })?.orderOut(nil)
                    ClipboardPanelController.shared.toggle()
                }
            }
            #endif
            CommandMenu("共享") {
                Button("文件暂存区") { store.shelf?.show() }.keyboardShortcut("d", modifiers: [.command, .shift])
                Button("立即收回键鼠") { store.reclaim() }.keyboardShortcut(.escape, modifiers: [.control, .option, .command])
                    .disabled(!(store.snapshot?.input.managed == true || store.snapshot?.input.settings.enabled == true))
                Divider()
                Button("刷新设备") { store.send(.refreshDevices) }
            }
            // 系统默认的「DeskFuse 帮助」没有帮助手册，只会弹出“帮助不可用”；换成真正能联系到开发者的入口。
            CommandGroup(replacing: .help) {
                Button("反馈问题…") { SupportCenter.reportProblem(store) }
                    .keyboardShortcut("?", modifiers: .command)
                Button("导出诊断信息…") { SupportCenter.exportDiagnosticsInteractively(store) }
                    .disabled(store.client == nil)
                Button("复制版本与环境信息") { SupportCenter.copyEnvironment(store) }
                Divider()
                Button("已报告的问题") { SupportCenter.open(SupportCenter.issuesURL) }
                Button("发行说明") { SupportCenter.open(SupportCenter.releasesURL) }
                Button("安全问题反馈") { SupportCenter.open(SupportCenter.securityURL) }
            }
        }


    }

    #if DEBUG
    /// Reproducible point-sized visual checks, using the real window and engine.
    /// These menu items are not compiled into distributable Release builds.
    private func resizeForDesign(width: CGFloat, height: CGFloat) {
        guard let window = NSApp.windows.first(where: { $0.identifier?.rawValue == "deskfuse-main" }) else { return }
        var frame = window.frame
        frame.origin.y += frame.height - height
        frame.size = NSSize(width: width, height: height)
        window.setFrame(frame, display: true)
        window.makeKeyAndOrderFront(nil)
    }
    #endif

}
@MainActor final class AppDelegate: NSObject, NSApplicationDelegate {
    private var instanceLockFD: Int32 = -1

    func applicationDidFinishLaunching(_ notification: Notification) {
        // NSRunningApplication enumeration alone has a startup race: two
        // launches can both observe no peer and both keep their Dock icon.
        // An advisory lock is released by the kernel when the owning process
        // exits, so it does not leave a stale PID marker after a crash.
        guard acquireInstanceLock() else {
            // System Settings may launch the replacement before the old process
            // has released its lock. Give that handoff a bounded grace period.
            Task { @MainActor in
                for _ in 0..<30 {
                    try? await Task.sleep(for: .milliseconds(100))
                    if acquireInstanceLock() {
                        finishLaunching()
                        return
                    }
                }
                activateExistingInstance()
                NSApp.terminate(nil)
            }
            return
        }
        finishLaunching()
    }

    private func finishLaunching() {
        DockIconAppearanceController.shared.start()
        // The system controls activation; background App Intents must not steal focus.
        // This also applies the persisted Dock visibility choice before windows show.
        AppStore.shared.applyDockIconPolicy()
        AppStore.shared.previousInstallation.evaluate { [weak self] in
            self?.startEngine()
        }
    }

    private func startEngine() {
        AppStore.shared.start()
        let store = AppStore.shared
        // 自检流程随即退出，不弹任何引导窗口。
        guard !store.selfTest else {
            ClipboardPanelController.shared.install(store: store)
            return
        }
        // Opt-in only: a no-op unless launched with --mygo-ui <executable>.
        MyGoUIHost.shared.startIfRequested()
        // 开发期基线截图：仅在 DESKFUSE_CAPTURE_UI 下运行，随后自行退出。
        UICapture.runIfRequested(store: store)
        if store.onboarding.shouldPresent(), !store.previousInstallation.wasPresented {
            // 本机尚未完成当前引导：即使应用已经安装过，也运行一次。
            store.onboarding.present()
            if MyGoUIHost.shared.isActive { MyGoUIHost.shared.present(page: "onboarding") }
        } else {
            // 权限按需申请：功能页或设置页会在确有需要时提示。
            store.permissions.refresh()
        }
        ClipboardPanelController.shared.install(store: store)
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        MyGoUIHost.shared.stop()
        // 引导弹窗中授权后，系统设置的「退出并重新打开」已经进入退出流程。
        // 再确认模态窗口不拦截终止，避免 AppKit 在委托返回后丢掉这次退出。
        for window in NSApp.windows {
            if let sheet = window.attachedSheet {
                sheet.preventsApplicationTerminationWhenModal = false
            }
            if window.isSheet {
                window.preventsApplicationTerminationWhenModal = false
            }
        }
        return AppStore.shared.terminate()
    }
    func applicationDockMenu(_ sender: NSApplication) -> NSMenu? {
        if #available(macOS 14.2, *) {
            return RemoteApplicationDockController.shared.makeDockMenu()
        }
        return nil
    }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        if #available(macOS 14.2, *), RemoteApplicationManager.shared.focusViewer() { return true }
        AppStore.shared.openMain()
        return true
    }

    private func runningInstance() -> NSRunningApplication? {
        NSRunningApplication.runningApplications(withBundleIdentifier: "com.deskfuse.app")
            .first { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier }
    }

    private func activateExistingInstance() {
        runningInstance()?.activate(options: [.activateAllWindows, .activateIgnoringOtherApps])
    }

    private func acquireInstanceLock() -> Bool {
        let manager = FileManager.default
        guard let support = manager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else { return false }
        let directory = support.appendingPathComponent("DeskFuse", isDirectory: true)
        do { try manager.createDirectory(at: directory, withIntermediateDirectories: true) } catch { return false }
        let path = directory.appendingPathComponent("instance.lock").path
        // Helpers must not retain the app's lock after the main process exits.
        let fd = open(path, O_CREAT | O_RDWR | O_CLOEXEC, S_IRUSR | S_IWUSR)
        guard fd >= 0 else { return false }
        guard flock(fd, LOCK_EX | LOCK_NB) == 0 else {
            close(fd)
            return false
        }
        instanceLockFD = fd
        return true
    }

}
/// Assigns window identity and presentation without removing the native
/// titlebar, its traffic lights, or its drag region.
@MainActor
final class WindowChromeMarker: NSView {
    var windowIdentifier: NSUserInterfaceItemIdentifier?
    var minimumSize: NSSize?

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard let window else { return }

        if let windowIdentifier { window.identifier = windowIdentifier }
        if let minimumSize { window.minSize = minimumSize; window.isReleasedWhenClosed = false }
        window.styleMask.formUnion([.titled, .closable, .miniaturizable, .resizable])
        window.backgroundColor = .windowBackgroundColor
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.titlebarSeparatorStyle = .none
        observeTitlebarLayout(window)
        layoutTrafficLights()
    }

    override func layout() {
        super.layout()
        layoutTrafficLights()
    }

    /// 只对主窗口生效：把标题栏容器加高到 `DFMetrics.titlebarHeight`，红绿灯在这一行内垂直居中，
    /// 品牌位与页面面包屑按同一高度对齐。不用 NSToolbar：macOS 26 会把内容托管进玻璃工具栏并加滚动边缘效果。
    private func layoutTrafficLights() {
        guard windowIdentifier != nil, let window, !window.styleMask.contains(.fullScreen),
              let close = window.standardWindowButton(.closeButton),
              let miniaturize = window.standardWindowButton(.miniaturizeButton),
              let zoom = window.standardWindowButton(.zoomButton),
              let container = close.superview?.superview else { return }
        let height = DFMetrics.titlebarHeight
        let containerFrame = NSRect(x: 0, y: window.frame.height - height, width: window.frame.width, height: height)
        if container.frame != containerFrame { container.frame = containerFrame }
        let inset = (height - close.frame.height) / 2
        // 沿用系统按钮间距，只平移整组。
        let spacing = miniaturize.frame.minX - close.frame.minX
        for (index, button) in [close, miniaturize, zoom].enumerated() {
            let origin = NSPoint(x: inset + CGFloat(index) * spacing, y: inset)
            if button.frame.origin != origin { button.setFrameOrigin(origin) }
        }
    }

    private var titlebarObservers: [NSObjectProtocol] = []

    /// AppKit 会在缩放、切换 key 状态、退出全屏时重排标题栏，需要重新定位红绿灯。
    private func observeTitlebarLayout(_ window: NSWindow) {
        titlebarObservers.forEach(NotificationCenter.default.removeObserver)
        let names: [Notification.Name] = [NSWindow.didResizeNotification, NSWindow.didBecomeKeyNotification,
                                          NSWindow.didResignKeyNotification, NSWindow.didExitFullScreenNotification,
                                          NSWindow.didChangeScreenNotification]
        titlebarObservers = names.map { name in
            NotificationCenter.default.addObserver(forName: name, object: window, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.layoutTrafficLights() }
            }
        }
    }
}

struct WindowIdentity: NSViewRepresentable {
    func makeNSView(context: Context) -> WindowChromeMarker {
        let marker = WindowChromeMarker()
        marker.windowIdentifier = NSUserInterfaceItemIdentifier("deskfuse-main")
        marker.minimumSize = NSSize(width: 960, height: 620)
        return marker
    }
    func updateNSView(_ view: WindowChromeMarker, context: Context) { }
}
