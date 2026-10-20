import Foundation
import Darwin

/// Exercises the real ditto/codesign transaction using disposable signed app bundles.
@main
enum AppInstallationTests {
    struct InjectedFailure: Error {}

    static func main() throws {
        let manager = FileManager.default
        let root = manager.temporaryDirectory.appendingPathComponent("deskfuse-installer-tests-\(UUID())")
        try manager.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? manager.removeItem(at: root) }
        if let path = CommandLine.arguments.dropFirst().first {
            let mountedApp = URL(fileURLWithPath: path)
            try expect(AppInstallation.requiresInstallation(bundleURL: mountedApp), "DMG enters installer")
            let installed = root.appendingPathComponent("Applications/DeskFuse.app")
            try AppInstallation.install(source: mountedApp, destination: installed)
            try expect(!AppInstallation.requiresInstallation(
                bundleURL: installed, applicationDirectories: [installed.deletingLastPathComponent()]),
                "installed copy enters main app")
            print("DMG integration: read-only entry, signed bundle copy, installed entry passed")
            return
        }
        let source = root.appendingPathComponent("Source.app")
        try fixture(source, version: "new")
        let destination = root.appendingPathComponent("Applications/DeskFuse.app")
        try AppInstallation.install(source: source, destination: destination)
        try expect(contents(destination) == "new", "fresh installation")
        try quarantinedInstallationTest(source: source, root: root)

        try manager.removeItem(at: destination)
        try fixture(destination, version: "old")
        do {
            try AppInstallation.install(source: source, destination: destination,
                                        validate: { _ in throw InjectedFailure() })
            fatalError("verification failure was ignored")
        } catch is InjectedFailure {}
        try expect(contents(destination) == "old", "verification failure preserves old app")

        let damaged = root.appendingPathComponent("Damaged.app")
        try manager.copyItem(at: source, to: damaged)
        try "tampered".write(to: damaged.appendingPathComponent("Contents/Resources/version"),
                            atomically: true, encoding: .utf8)
        do {
            try AppInstallation.install(source: damaged, destination: destination)
            fatalError("damaged signature was accepted")
        } catch InstallationFailure.commandFailed {}
        try expect(contents(destination) == "old", "damaged signature preserves old app")

        do {
            try AppInstallation.install(source: source, destination: destination, copy: { _, _ in
                try AppInstallation.install(source: source, destination: destination)
            })
            fatalError("concurrent installation was accepted")
        } catch InstallationFailure.alreadyInstalling {}
        try expect(contents(destination) == "old", "concurrent installer leaves old app intact")

        do {
            try AppInstallation.install(source: source, destination: destination, move: { from, to in
                if from.lastPathComponent == "DeskFuse.app", from != destination { throw InjectedFailure() }
                try manager.moveItem(at: from, to: to)
            })
            fatalError("replacement failure was ignored")
        } catch is InjectedFailure {}
        try expect(contents(destination) == "old", "replacement failure restores old app")

        try AppInstallation.install(source: source, destination: destination)
        try expect(contents(destination) == "new", "replaces existing signed app")
        try expect(try manager.contentsOfDirectory(atPath: destination.deletingLastPathComponent().path)
            .allSatisfy { !$0.hasPrefix(".DeskFuse-install-") }, "successful transactions clean staging")

        let target = root.appendingPathComponent("Linked/DeskFuse.app")
        try manager.createDirectory(at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
        try manager.createSymbolicLink(at: target, withDestinationURL: destination)
        do {
            try AppInstallation.install(source: source, destination: target)
            fatalError("symlink destination was accepted")
        } catch InstallationFailure.invalidDestination {}
        try expect(contents(destination) == "new", "symlink target remains untouched")

        do {
            try AppInstallation.install(source: source, destination: destination, move: { from, to in
                if from != destination { throw InjectedFailure() }
                try manager.moveItem(at: from, to: to)
            })
            fatalError("restore failure was ignored")
        } catch InstallationFailure.restoreFailed(let backup) {
            try expect(contents(URL(fileURLWithPath: backup)) == "new", "failed rollback preserves recovery copy")
        }
        try expect(!AppInstallation.requiresInstallation(bundleURL: source), "writable app bypasses installer")
        try expect(AppInstallation.requiresInstallation(
            bundleURL: URL(fileURLWithPath: "/private/tmp/AppTranslocation/id/d/DeskFuse.app")),
            "translocated app enters installer")
        try expect(!AppInstallation.requiresInstallation(
            bundleURL: URL(fileURLWithPath: "/tmp/deskfuse-app")), "development executable bypasses installer")
        try expect(AppInstallation.requiresInstallation(
            bundleURL: manager.homeDirectoryForCurrentUser.appendingPathComponent("Downloads/DeskFuse.app")),
            "downloaded app enters installer without quarantine")
        try expect(!AppInstallation.requiresInstallation(
            bundleURL: URL(fileURLWithPath: "/Applications/DeskFuse.app")), "system installation starts normally")
        try expect(!AppInstallation.requiresInstallation(
            bundleURL: manager.homeDirectoryForCurrentUser.appendingPathComponent("Applications/DeskFuse.app")),
            "user installation starts normally")
        try diskImageTest(source: source, root: root)
        print("Installation transaction checks passed, including quarantine, direct app entry and a real read-only DMG")
    }

    private static func quarantinedInstallationTest(source: URL, root: URL) throws {
        let manager = FileManager.default
        let download = root.appendingPathComponent("Downloads/DeskFuse.app")
        try manager.createDirectory(at: download.deletingLastPathComponent(), withIntermediateDirectories: true)
        try manager.copyItem(at: source, to: download)
        let executable = download.appendingPathComponent("Contents/MacOS/test-app")
        for path in [download.path, executable.path] {
            try run("/usr/bin/xattr", ["-w", "com.apple.quarantine", "00c2;00000000;Keka;", path])
        }
        try run("/usr/bin/xattr", ["-w", "cc.deskfuse.installer-test", "preserved", download.path])
        let installed = root.appendingPathComponent("QuarantineApplications/DeskFuse.app")
        try AppInstallation.install(source: download, destination: installed)
        for url in [installed, installed.appendingPathComponent("Contents/MacOS/test-app")] {
            try expect(!hasQuarantine(url),
                       "installed bundle and executable do not inherit download quarantine")
        }
        try expect(hasQuarantine(download),
                   "original download retains quarantine")
        try run("/usr/bin/xattr", ["-p", "cc.deskfuse.installer-test", installed.path])
        try run("/usr/bin/codesign", ["--verify", "--deep", "--strict", installed.path])
        try expect(!AppInstallation.requiresInstallation(
            bundleURL: installed, applicationDirectories: [installed.deletingLastPathComponent()]),
            "quarantined download becomes a normal installed application")
    }

    private static func hasQuarantine(_ url: URL) -> Bool {
        getxattr(url.path, "com.apple.quarantine", nil, 0, 0, XATTR_NOFOLLOW) >= 0
    }

    private static func diskImageTest(source: URL, root: URL) throws {
        let manager = FileManager.default
        let imageRoot = root.appendingPathComponent("image-root")
        let mount = root.appendingPathComponent("mounted")
        try manager.createDirectory(at: imageRoot, withIntermediateDirectories: true)
        try manager.createDirectory(at: mount, withIntermediateDirectories: true)
        try manager.copyItem(at: source, to: imageRoot.appendingPathComponent("DeskFuse.app"))
        let image = root.appendingPathComponent("fixture.dmg")
        try run("/usr/bin/hdiutil", ["create", "-quiet", "-srcfolder", imageRoot.path,
                                    "-format", "UDZO", "-volname", "DeskFuseInstallerTest", image.path])
        try run("/usr/bin/hdiutil", ["attach", "-quiet", "-nobrowse", "-readonly",
                                    "-mountpoint", mount.path, image.path])
        defer { try? run("/usr/bin/hdiutil", ["detach", "-quiet", mount.path]) }
        let mountedApp = mount.appendingPathComponent("DeskFuse.app")
        try expect(AppInstallation.requiresInstallation(bundleURL: mountedApp), "read-only DMG enters installer")
        let installed = root.appendingPathComponent("image-installed/DeskFuse.app")
        try AppInstallation.install(source: mountedApp, destination: installed)
        try expect(contents(installed) == "new", "signed application copied off DMG")
        try expect(!AppInstallation.requiresInstallation(
            bundleURL: installed, applicationDirectories: [installed.deletingLastPathComponent()]),
            "installed copy enters main app")
    }

    private static func run(_ executable: String, _ arguments: [String]) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        try process.run()
        process.waitUntilExit()
        precondition(process.terminationStatus == 0, "\(executable) failed")
    }

    private static func expect(_ condition: @autoclosure () throws -> Bool, _ message: String) throws {
        guard try condition() else { fatalError(message) }
    }

    private static func contents(_ app: URL) throws -> String {
        try String(contentsOf: app.appendingPathComponent("Contents/Resources/version"), encoding: .utf8)
    }

    private static func fixture(_ app: URL, version: String) throws {
        let manager = FileManager.default
        try manager.createDirectory(at: app.appendingPathComponent("Contents/MacOS"), withIntermediateDirectories: true)
        try manager.createDirectory(at: app.appendingPathComponent("Contents/Resources"), withIntermediateDirectories: true)
        try manager.copyItem(atPath: "/usr/bin/true", toPath: app.appendingPathComponent("Contents/MacOS/test-app").path)
        let info = ["CFBundleIdentifier": "com.deskfuse.app", "CFBundleExecutable": "test-app",
                    "CFBundlePackageType": "APPL", "CFBundleVersion": "1"]
        try PropertyListSerialization.data(fromPropertyList: info, format: .xml, options: 0)
            .write(to: app.appendingPathComponent("Contents/Info.plist"))
        try version.write(to: app.appendingPathComponent("Contents/Resources/version"),
                          atomically: true, encoding: .utf8)
        let sign = Process()
        sign.executableURL = URL(fileURLWithPath: "/usr/bin/codesign")
        sign.arguments = ["--force", "--sign", "-", app.path]
        sign.standardError = FileHandle.nullDevice
        try sign.run()
        sign.waitUntilExit()
        precondition(sign.terminationStatus == 0, "fixture signing failed")
    }
}
