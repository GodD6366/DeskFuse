import SwiftUI
import AppKit
import UniDeskEngine

@main @MainActor struct UniDeskApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @State private var store = AppStore.shared
    var body: some SwiftUI.Scene {
        Window("uniDesk", id: "main") {
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
        guard let window = NSApp.windows.first(where: { $0.identifier?.rawValue == "unidesk-main" }) else { return }
        var frame = window.frame
        frame.origin.y += frame.height - height
        frame.size = NSSize(width: width, height: height)
        window.setFrame(frame, display: true)
        window.makeKeyAndOrderFront(nil)
    }
    #endif

}
@MainActor final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        if let other = NSRunningApplication.runningApplications(withBundleIdentifier: "com.unidesk.app").first(where: { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier }) {
            other.activate(options: [.activateAllWindows]); NSApp.terminate(nil); return
        }
        // The system controls activation; background App Intents must not steal focus.
        // This also applies the persisted Dock visibility choice before windows show.
        AppStore.shared.applyDockIconPolicy()
        AppStore.shared.start()
        ClipboardPanelController.shared.install(store: AppStore.shared)
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply { AppStore.shared.terminate() }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool { AppStore.shared.openMain(); return true }
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
        marker.windowIdentifier = NSUserInterfaceItemIdentifier("unidesk-main")
        marker.minimumSize = NSSize(width: 960, height: 620)
        return marker
    }
    func updateNSView(_ view: WindowChromeMarker, context: Context) { }
}

struct SettingsWindowChrome: NSViewRepresentable {
    func makeNSView(context: Context) -> WindowChromeMarker { WindowChromeMarker() }
    func updateNSView(_ view: WindowChromeMarker, context: Context) { }
}
