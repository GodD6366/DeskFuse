import Foundation
import Darwin

/// File operations are independent of the main application and never touch user data.
enum AppInstallation {
    static func requiresInstallation(bundleURL: URL, applicationDirectories: [URL]? = nil) -> Bool {
        guard bundleURL.pathExtension == "app" else { return false }
        // Gatekeeper may relocate a quarantined app out of its original disk image.
        if bundleURL.pathComponents.contains("AppTranslocation") { return true }
        if (try? bundleURL.resourceValues(forKeys: [.volumeIsReadOnlyKey]))?.volumeIsReadOnly == true {
            return true
        }
        // A directly distributed .app needs the same installer even without
        // quarantine/App Translocation. Named development bundles remain portable.
        guard bundleURL.lastPathComponent == "DeskFuse.app" else { return false }
        let directories = applicationDirectories
            ?? FileManager.default.urls(for: .applicationDirectory, in: .allDomainsMask)
        let path = bundleURL.resolvingSymlinksInPath().standardizedFileURL.path
        return !directories.contains { directory in
            path.hasPrefix(directory.resolvingSymlinksInPath().standardizedFileURL.path + "/")
        }
    }

    static func destination(fileManager: FileManager = .default) -> URL {
        let applications = URL(fileURLWithPath: "/Applications", isDirectory: true)
        let systemApp = applications.appendingPathComponent("DeskFuse.app")
        // Never shadow an existing system installation with a second user copy.
        if fileManager.fileExists(atPath: systemApp.path) ||
            fileManager.isWritableFile(atPath: applications.path) { return systemApp }
        return fileManager.homeDirectoryForCurrentUser
            .appendingPathComponent("Applications/DeskFuse.app")
    }

    /// Stage and verify the entire signed bundle before moving the old app.
    /// The injected operations let regression tests exercise rollback without /Applications.
    static func install(
        source: URL, destination: URL,
        copy: (URL, URL) throws -> Void = copyBundle,
        validate: (URL) throws -> Void = validateBundle,
        move: (URL, URL) throws -> Void = { try FileManager.default.moveItem(at: $0, to: $1) }
    ) throws {
        let manager = FileManager.default
        let parent = destination.deletingLastPathComponent()
        try manager.createDirectory(at: parent, withIntermediateDirectories: true)
        // Serialize installs from multiple mounted DMGs. This is separate from the
        // installed app's instance lock, which must remain free during installation.
        let lock = parent.appendingPathComponent(".deskfuse-install.lock")
        let fd = open(lock.path, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, S_IRUSR | S_IWUSR)
        guard fd >= 0 else { throw InstallationFailure.destinationNotWritable }
        defer { close(fd) }
        guard flock(fd, LOCK_EX | LOCK_NB) == 0 else { throw InstallationFailure.alreadyInstalling }
        guard source.standardizedFileURL != destination.standardizedFileURL,
              (try? destination.resourceValues(forKeys: [.isSymbolicLinkKey]))?.isSymbolicLink != true
        else { throw InstallationFailure.invalidDestination }
        if manager.fileExists(atPath: destination.path) {
            guard Bundle(url: destination)?.bundleIdentifier == "com.deskfuse.app" else {
                throw InstallationFailure.invalidDestination
            }
        }

        let transaction = parent.appendingPathComponent(".DeskFuse-install-\(UUID().uuidString)")
        try manager.createDirectory(at: transaction, withIntermediateDirectories: false)
        let staged = transaction.appendingPathComponent("DeskFuse.app")
        let backup = transaction.appendingPathComponent("Previous.app")
        var preserveBackup = false
        defer { if !preserveBackup { try? manager.removeItem(at: transaction) } }
        try copy(source, staged)
        try validate(staged)
        // The user has launched this installer and explicitly chosen Install.
        // Only after verifying the staged signature, remove inherited download
        // quarantine from that copy. Otherwise Launch Services translocates the
        // installed app again, sending it back through the installer entrypoint.
        // Keep all other metadata and the original download untouched.
        try run("/usr/bin/xattr", ["-r", "-s", "-d", "com.apple.quarantine", staged.path])

        let replacing = manager.fileExists(atPath: destination.path)
        if replacing { try move(destination, backup) }
        do {
            try move(staged, destination)
        } catch {
            if replacing {
                do { try move(backup, destination) }
                catch {
                    preserveBackup = true
                    throw InstallationFailure.restoreFailed(backup.path)
                }
            }
            throw error
        }
    }

    private static func copyBundle(_ source: URL, _ destination: URL) throws {
        // Preserve all metadata until the staged signature has been verified.
        try run("/usr/bin/ditto", [source.path, destination.path])
    }

    private static func validateBundle(_ app: URL) throws {
        guard let bundle = Bundle(url: app),
              bundle.bundleIdentifier == "com.deskfuse.app",
              let executable = bundle.executableURL,
              FileManager.default.isExecutableFile(atPath: executable.path) else {
            throw InstallationFailure.invalidBundle
        }
        try run("/usr/bin/codesign", ["--verify", "--deep", "--strict", app.path])
    }

    private static func run(_ executable: String, _ arguments: [String]) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        let pipe = Pipe()
        process.standardError = pipe
        try process.run()
        let output = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw InstallationFailure.commandFailed(String(decoding: output, as: UTF8.self))
        }
    }
}

enum InstallationFailure: Error {
    case destinationNotWritable, alreadyInstalling, invalidDestination, invalidBundle
    case runningApplication, restoreFailed(String), commandFailed(String)

    func message(english: Bool) -> String {
        switch self {
        case .destinationNotWritable:
            english ? "You don’t have permission to update this application. Ask an administrator to install DeskFuse."
                : "没有权限写入应用程序文件夹，请联系管理员安装 DeskFuse。"
        case .alreadyInstalling:
            english ? "Another DeskFuse installation is in progress. Try again when it finishes."
                : "另一个 DeskFuse 安装正在进行，请等待完成后重试。"
        case .invalidDestination:
            english ? "The destination is not a DeskFuse application. Check the application in Finder."
                : "目标位置不是有效的 DeskFuse 应用，请在访达中检查。"
        case .invalidBundle:
            english ? "The installation package is incomplete. Download DeskFuse again."
                : "安装包不完整，请重新下载 DeskFuse。"
        case .runningApplication:
            english ? "DeskFuse is still running. Quit it, then try again."
                : "DeskFuse 尚未退出，请退出后重试。"
        case .restoreFailed(let path):
            english ? "The previous version is saved at \(path). Restore it in Finder before retrying."
                : "旧版本保留在 \(path)，请在访达中恢复后重试。"
        case .commandFailed(let details):
            english ? "Couldn’t copy or verify DeskFuse. \(details)"
                : "无法复制或验证 DeskFuse。\(details)"
        }
    }
}

