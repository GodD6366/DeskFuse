import SwiftUI
import AppKit
import Darwin
import DeskFuseEngine

@MainActor struct DeskFuseApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var store = AppStore.shared
    /// 产品窗口由 WebView 客户端提供，宿主不创建任何窗口场景。
    /// 这里保留 commands，让菜单栏（⌘,、共享、帮助）仍然属于主程序。
    var body: some SwiftUI.Scene {
        Settings {
            EmptyView()
        }
        .commands {
            CommandGroup(replacing: .newItem) { }
            CommandGroup(replacing: .appSettings) {
                Button("设置…") { store.openSettings() }
                    .keyboardShortcut(",", modifiers: .command)
            }
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
