import AppKit
import Foundation

/// 开发期界面基线：把原生窗口逐页渲染成 PNG，用来和 WebView 版逐页比对。
/// 仅由 `DESKFUSE_CAPTURE_UI=<目录>` 触发；不参与正常启动，也不改动业务状态。
@MainActor enum UICapture {
    static var directory: URL? {
        guard let path = ProcessInfo.processInfo.environment["DESKFUSE_CAPTURE_UI"], !path.isEmpty else { return nil }
        return URL(fileURLWithPath: path, isDirectory: true)
    }

    static func runIfRequested(store: AppStore) {
        guard let directory else { return }
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        Task { @MainActor in
            guard await waitForSnapshot(store) else {
                NSLog("UI capture: no snapshot")
                NSApp.terminate(nil)
                return
            }
            guard let window = NSApp.windows.first(where: { $0.identifier?.rawValue == "deskfuse-main" }) else {
                NSLog("UI capture: no main window")
                NSApp.terminate(nil)
                return
            }
            window.setContentSize(NSSize(width: 1120, height: 720))
            window.makeKeyAndOrderFront(nil)
            for page in Page.allCases {
                store.selectedPage = page
                try? await Task.sleep(for: .milliseconds(700))
                capture(window, to: directory.appendingPathComponent("\(page.bridgeID).png"))
                if page == .settings {
                    for category in SettingsCategory.allCases {
                        store.settingsDestination = SettingsDestination(tab: category.rawValue, previous: store.settingsDestination)
                        try? await Task.sleep(for: .milliseconds(500))
                        capture(window, to: directory.appendingPathComponent("settings-\(category.rawValue).png"))
                    }
                }
            }
            NSLog("UI capture: wrote pages to %@", directory.path)
            NSApp.terminate(nil)
        }
    }

    /// 引擎先启动、后加入设备网络；等网络就绪再截图，避免把启动中间态当成基线。
    private static func waitForSnapshot(_ store: AppStore) async -> Bool {
        for _ in 0..<250 {
            if let snapshot = store.snapshot, snapshot.networkId != nil {
                try? await Task.sleep(for: .milliseconds(800))
                return true
            }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return false
    }

    private static func capture(_ window: NSWindow, to url: URL) {
        guard let view = window.contentView, view.bounds.width > 0,
              let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return }
        view.cacheDisplay(in: view.bounds, to: rep)
        guard let data = rep.representation(using: .png, properties: [:]) else { return }
        try? data.write(to: url)
    }
}
