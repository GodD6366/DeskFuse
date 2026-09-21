import SwiftUI
import AppKit
import Darwin
import DeskFuseEngine

@main @MainActor struct DeskFuseApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var store = AppStore.shared
    var body: some SwiftUI.Scene {
        Window("DeskFuse", id: "main") {
            RootView(store: store)
                .frame(minWidth: 960, maxWidth: .infinity, minHeight: 568, maxHeight: .infinity)
                .background(WindowIdentity())
        }
        .defaultSize(width: 1200, height: 780)
        .windowStyle(.titleBar)
        .commands {
            CommandGroup(replacing: .newItem) { }
            ClipboardCommands()
            #if DEBUG
            CommandGroup(after: .windowSize) {
                Button("标准设计窗口（1200 × 780）") { resizeForDesign(width: 1200, height: 780) }
                Button("紧凑设计窗口（960 × 620）") { resizeForDesign(width: 960, height: 620) }
            }
            #endif
            CommandMenu("共享") {
                Button("文件暂存区") { store.shelf?.show() }.keyboardShortcut("d", modifiers: [.command, .shift])
                Button("立即收回键鼠") { store.reclaim() }.keyboardShortcut(.escape, modifiers: [.control, .option, .command])
                Divider()
                Button("搜索设备") { store.send(.refreshDevices) }
            }
        }

        Settings {
            SettingsWindowView(store: store)
                .frame(minWidth: 860, idealWidth: 960, maxWidth: .infinity,
                       minHeight: 580, idealHeight: 720, maxHeight: .infinity)
                .background(SettingsWindowChrome())
        }
        .defaultSize(width: 960, height: 720)
        .windowStyle(.titleBar)
        .windowResizability(.contentMinSize)
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

    func applicationWillFinishLaunching(_ notification: Notification) {
        TransientScrollbars.shared.install()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        // NSRunningApplication enumeration alone has a startup race: two
        // launches can both observe no peer and both keep their Dock icon.
        // An advisory lock is released by the kernel when the owning process
        // exits, so it does not leave a stale PID marker after a crash.
        guard acquireInstanceLock() else {
            activateExistingInstance()
            NSApp.terminate(nil)
            return
        }
        // The system controls activation; background App Intents must not steal focus.
        // This also applies the persisted Dock visibility choice before windows show.
        AppStore.shared.applyDockIconPolicy()
        AppStore.shared.start()
        // 自检流程随即退出，不弹权限窗口；其余启动在缺少权限时提示一次。
        if !AppStore.shared.selfTest { AppStore.shared.permissionPrompt.evaluateAtLaunch() }
        ClipboardPanelController.shared.install(store: AppStore.shared)
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply { AppStore.shared.terminate() }
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

struct SettingsWindowChrome: NSViewRepresentable {
    func makeNSView(context: Context) -> WindowChromeMarker { WindowChromeMarker() }
    func updateNSView(_ view: WindowChromeMarker, context: Context) { }
}
