import Foundation
import Testing
@testable import DeskFuse

struct DeskFuseUninstallerTests {
    private typealias Component = DeskFuseUninstaller.Component

    private func plan(home: URL, bundle: URL, existing: Set<String> = []) -> DeskFuseUninstaller.Plan {
        .make(home: home, temporaryDirectory: home.appendingPathComponent("tmp"),
              bundleURL: bundle, fileExists: existing.contains)
    }

    @Test func clipboardDataIsKeptByDefault() {
        let home = URL(fileURLWithPath: "/Users/tester")
        let plan = plan(home: home, bundle: URL(fileURLWithPath: "/Applications/DeskFuse.app"))
        #expect(!plan.defaultSelection.contains(.userData))
        #expect(plan.defaultSelection.contains(.caches))
        let paths = plan.paths(for: plan.defaultSelection).map(\.path)
        #expect(!paths.contains(plan.userData.path))
        // 默认只删除数据目录里的更新下载，不碰剪贴板数据库与 blobs。
        #expect(paths.contains("/Users/tester/Library/Application Support/DeskFuse/updates"))
        #expect(!paths.contains { $0.hasSuffix("state.sqlite3") || $0.hasSuffix("/blobs") })
    }

    @Test func pathsStayInsideDeskFuseLocations() {
        let home = URL(fileURLWithPath: "/Users/tester")
        let plan = plan(home: home, bundle: URL(fileURLWithPath: "/Applications/DeskFuse.app"))
        let paths = plan.paths(for: Set(Component.allCases)).map(\.path)
        #expect(paths.allSatisfy { $0.hasPrefix("/Users/tester/Library/") })
        #expect(paths.allSatisfy { $0.localizedCaseInsensitiveContains("deskfuse") })
        // 发布工具链的签名密钥目录绝不能被卸载删除。
        #expect(!paths.contains { $0.contains("DeskFuse-release") })
        #expect(!paths.contains("/Users/tester/Library/Application Support"))
    }

    @Test func optionalComponentsDependOnInstallation() {
        let home = URL(fileURLWithPath: "/Users/tester")
        let bare = plan(home: home, bundle: URL(fileURLWithPath: "/tmp/build/debug"))
        #expect(!bare.availableComponents.contains(.application))
        #expect(!bare.availableComponents.contains(.microphoneDriver))
        #expect(!bare.installedByHomebrew)

        let full = plan(home: home, bundle: URL(fileURLWithPath: "/Applications/DeskFuse.app"),
                        existing: [MicrophoneDriverManager.installedPath, "/opt/homebrew/Caskroom/deskfuse"])
        #expect(full.availableComponents.contains(.application))
        #expect(full.availableComponents.contains(.microphoneDriver))
        #expect(full.installedByHomebrew)
    }

    @Test func unselectedItemsAreNotPassedToCleanup() {
        let home = URL(fileURLWithPath: "/Users/tester")
        let plan = plan(home: home, bundle: URL(fileURLWithPath: "/Applications/DeskFuse.app"))
        let arguments = DeskFuseUninstaller.cleanupArguments(plan, selection: [.keychain], pid: 42)
        // $1 PID、$2 应用、$3 临时目录；未勾选时后两者为空，且没有任何待删除路径。
        #expect(Array(arguments.suffix(3)) == ["42", "", ""])
        let environment = DeskFuseUninstaller.cleanupEnvironment(plan, selection: [.keychain])
        #expect(environment["DESKFUSE_PREFERENCE_DOMAINS"] == "")
    }

    /// 在临时 HOME 中真实运行清理脚本：等待目标进程退出后删除所选目录、
    /// 只清理 `deskfuse-` 前缀的临时文件，并把应用移到废纸篓。
    @Test func cleanupScriptRemovesSelectionAfterProcessExits() throws {
        let fm = FileManager.default
        let home = fm.temporaryDirectory.appendingPathComponent("deskfuse-uninstall-test-\(UUID().uuidString)")
        defer { try? fm.removeItem(at: home) }
        let bundle = home.appendingPathComponent("Apps/DeskFuse.app")
        let plan = plan(home: home, bundle: bundle)
        let release = home.appendingPathComponent("Library/Application Support/DeskFuse-release")
        for directory in [plan.userData, release, bundle, plan.temporaryDirectory] + plan.caches {
            try fm.createDirectory(at: directory, withIntermediateDirectories: true)
        }
        let database = plan.userData.appendingPathComponent("state.sqlite3")
        try Data("db".utf8).write(to: database)
        try Data().write(to: plan.temporaryDirectory.appendingPathComponent("deskfuse-preview-1.png"))
        let unrelated = plan.temporaryDirectory.appendingPathComponent("other.txt")
        try Data().write(to: unrelated)

        // 模拟仍在运行的应用：脚本必须等它退出后才删除。
        let app = Process()
        app.executableURL = URL(fileURLWithPath: "/bin/sleep")
        app.arguments = ["0.3"]
        try app.run()

        let selection: Set<Component> = [.caches, .application]
        let arguments = DeskFuseUninstaller.cleanupArguments(plan, selection: selection, pid: app.processIdentifier)
        let script = Process()
        script.executableURL = URL(fileURLWithPath: arguments[0])
        script.arguments = Array(arguments.dropFirst())
        var environment = DeskFuseUninstaller.cleanupEnvironment(plan, selection: selection)
        environment["HOME"] = home.path
        script.environment = environment
        try script.run()
        script.waitUntilExit()

        #expect(script.terminationStatus == 0)
        #expect(!app.isRunning)
        #expect(plan.caches.allSatisfy { !fm.fileExists(atPath: $0.path) })
        #expect(fm.fileExists(atPath: database.path), "未勾选时剪贴板数据库必须保留")
        #expect(fm.fileExists(atPath: release.path))
        #expect(!fm.fileExists(atPath: plan.temporaryDirectory.appendingPathComponent("deskfuse-preview-1.png").path))
        #expect(fm.fileExists(atPath: unrelated.path))
        #expect(!fm.fileExists(atPath: bundle.path))
        #expect(fm.fileExists(atPath: home.appendingPathComponent(".Trash/DeskFuse.app").path))
    }
}
