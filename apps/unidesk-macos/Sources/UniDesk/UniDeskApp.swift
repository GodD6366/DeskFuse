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
        .windowToolbarStyle(.unified)
        .commands {
            CommandGroup(replacing: .newItem) { }
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
        Settings { SettingsView(store: store).frame(width: 880, height: 650) }
        MenuBarExtra {
            MenuBarClipboardView(store: store)
        } label: {
            Image(nsImage: Self.menuBarIcon)
                .renderingMode(.template)
                .accessibilityLabel("uniDesk")
        }
        .menuBarExtraStyle(.window)
    }
    /// Load once at the menu bar's logical size; the bundled PNG carries 2x pixels.
    /// Template rendering lets macOS handle light, dark and selected appearances.
    private static let menuBarIcon: NSImage = {
        let image = Bundle.main.url(forResource: "unidesk-status-icon", withExtension: "png")
            .flatMap { NSImage(contentsOf: $0) }
            ?? NSImage(systemSymbolName: "rectangle.on.rectangle", accessibilityDescription: "uniDesk")
            ?? NSImage(size: NSSize(width: 18, height: 18))
        image.size = NSSize(width: 18, height: 18)
        image.isTemplate = true
        return image
    }()

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
        NSApp.setActivationPolicy(.regular)
        // The system controls activation; background App Intents must not steal focus.
        AppStore.shared.start()
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply { AppStore.shared.terminate() }
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool { AppStore.shared.openMain(); return true }
}
struct WindowIdentity: NSViewRepresentable {
    final class Marker: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            window?.identifier = NSUserInterfaceItemIdentifier("unidesk-main")
            window?.minSize = NSSize(width: 960, height: 620)
            window?.isReleasedWhenClosed = false
        }
    }
    func makeNSView(context: Context) -> Marker { Marker() }
    func updateNSView(_ view: Marker, context: Context) { }
}
