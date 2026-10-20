import AppKit
import SwiftUI

/// Compiled by preview-installer.sh; never included in the distributable application.
@main
@MainActor
enum InstallerPreview {
    static func main() throws {
        let arguments = CommandLine.arguments
        guard arguments.count == 4 else { fatalError("expected icon path, output directory, and state") }
        let output = URL(fileURLWithPath: arguments[2])
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        app.appearance = NSAppearance(named: .aqua)
        let states: [(String, InstallationModel.Phase, Bool)] = [
            ("welcome", .welcome, false), ("installing", .installing, false),
            ("installed", .installed, false), ("welcome-en", .welcome, true),
            ("failed", .failed, false),
        ]
        for (name, phase, english) in states where name == arguments[3] {
            let model = InstallationModel(icon: NSImage(contentsOfFile: arguments[1]))
            model.phase = phase
            model.english = english
            model.failure = phase == .failed ? "暂时无法写入应用程序文件夹，请检查权限后重试。" : nil
            // A fresh native host captures AppKit-backed Link/ProgressView as well.
            // Reusing a host can capture only SwiftUI's dirty region after a state change.
            let controller = InstallationWindowController(model: model)
            controller.showWindow()
            guard let window = controller.window, let view = window.contentView else { fatalError("no window") }
            RunLoop.current.run(until: Date().addingTimeInterval(0.3))
            view.layoutSubtreeIfNeeded()
            guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { fatalError("no bitmap") }
            view.cacheDisplay(in: view.bounds, to: bitmap)
            guard let png = bitmap.representation(using: .png, properties: [:]) else { fatalError("no PNG") }
            try png.write(to: output.appendingPathComponent("\(name).png"))
            window.orderOut(nil)
        }
        print("Rendered native installer \(arguments[3]) to \(output.path)")
    }
}
