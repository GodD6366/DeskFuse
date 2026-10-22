import AppKit
import Darwin
import Foundation
import Security
import ServiceManagement

/// 彻底卸载 DeskFuse。
///
/// 需要系统 API 或管理员授权的步骤在应用内完成：麦克风驱动、登录项、钥匙串、系统权限记录。
/// 每一项都可在确认页单独勾选，剪贴板历史默认保留。
/// 数据目录、偏好设置与应用本体交给脱离会话的清理脚本，在本进程退出后再删除，
/// 避免仍在运行的引擎、窗口状态保存把文件重新写回来。
enum DeskFuseUninstaller {
    static let bundleIdentifier = "com.deskfuse.app"

    /// 可单独勾选的卸载项。
    enum Component: String, CaseIterable, Identifiable {
        case userData, caches, preferences, keychain, loginItem, privacy, microphoneDriver, application
        var id: String { rawValue }

        /// 剪贴板历史最重要：默认保留，用户明确勾选才删除。
        var selectedByDefault: Bool { self != .userData }
    }

    /// 卸载会清理的全部本机痕迹；纯数据，便于测试，也用于确认页逐项展示。
    struct Plan: Equatable {
        /// 剪贴板历史、配对记录、设备身份与设置所在目录。剪贴板与配对记录共用
        /// `state.sqlite3`，无法单独保留其一，因此整个目录作为一项。
        var userData: URL
        /// 缓存、日志、更新下载与窗口状态等可再生内容。
        var caches: [URL]
        var preferenceFiles: [URL]
        /// 退出后通过 `defaults delete` 清空的偏好设置域，避免 cfprefsd 缓存回写。
        var preferenceDomains: [String]
        var keychainServices: [String]
        /// 勾选缓存时，其中以 `deskfuse-` 开头的临时文件会被删除。
        var temporaryDirectory: URL
        /// 移到废纸篓的应用本体；从非 `.app` 路径运行（如 `swift run`）时为 nil。
        var appBundle: URL?
        /// 已安装、需要管理员授权才能移除的麦克风驱动。
        var microphoneDrivers: [String]
        /// 通过 Homebrew 安装时，Homebrew 自己的安装记录需要用户执行 `brew uninstall` 清理。
        var installedByHomebrew: Bool

        /// 本机实际存在、可以勾选的卸载项。
        var availableComponents: [Component] {
            Component.allCases.filter { component in
                switch component {
                case .microphoneDriver: !microphoneDrivers.isEmpty
                case .application: appBundle != nil
                default: true
                }
            }
        }

        var defaultSelection: Set<Component> {
            Set(availableComponents.filter(\.selectedByDefault))
        }

        /// 退出后由清理脚本删除的路径。
        func paths(for selection: Set<Component>) -> [URL] {
            var result: [URL] = []
            if selection.contains(.userData) { result.append(userData) }
            if selection.contains(.caches) { result += caches }
            if selection.contains(.preferences) { result += preferenceFiles }
            return result
        }

        static func make(home: URL, temporaryDirectory: URL, bundleURL: URL,
                         fileExists: (String) -> Bool) -> Plan {
            let library = home.appendingPathComponent("Library", isDirectory: true)
            let userData = library.appendingPathComponent("Application Support/DeskFuse", isDirectory: true)
            // 只列 DeskFuse 运行时写入的位置。`Application Support/DeskFuse-release` 是
            // 发布工具链的签名密钥与构建台账，不属于应用数据，绝不能被卸载删除。
            let caches = [userData.appendingPathComponent("updates", isDirectory: true)] + [
                "Caches/\(bundleIdentifier)",
                "Caches/deskfuse-ocr-helper",
                "Logs/DeskFuse",
                "HTTPStorages/\(bundleIdentifier)",
                "WebKit/\(bundleIdentifier)",
                "Saved Application State/\(bundleIdentifier).savedState",
            ].map { library.appendingPathComponent($0) }
            let preferenceFiles = [
                "Preferences/\(bundleIdentifier).plist",
                // 开发构建以裸可执行文件运行时使用进程名作为偏好设置域。
                "Preferences/deskfuse-app.plist",
            ].map { library.appendingPathComponent($0) }
            let drivers = [MicrophoneDriverManager.installedPath, MicrophoneDriverManager.legacyInstalledPath]
                .compactMap { $0 }
                .filter(fileExists)
            let homebrew = ["/opt/homebrew/Caskroom/deskfuse", "/usr/local/Caskroom/deskfuse"].contains(where: fileExists)
            return Plan(userData: userData,
                        caches: caches,
                        preferenceFiles: preferenceFiles,
                        preferenceDomains: [bundleIdentifier, "deskfuse-app"],
                        keychainServices: RemoteRDPKeychain.allServices,
                        temporaryDirectory: temporaryDirectory,
                        appBundle: bundleURL.pathExtension == "app" ? bundleURL : nil,
                        microphoneDrivers: drivers,
                        installedByHomebrew: homebrew)
        }

        static func current() -> Plan {
            make(home: FileManager.default.homeDirectoryForCurrentUser,
                 temporaryDirectory: FileManager.default.temporaryDirectory,
                 bundleURL: Bundle.main.bundleURL,
                 fileExists: { FileManager.default.fileExists(atPath: $0) })
        }
    }

    enum Failure: LocalizedError {
        case missingResources
        case cleanupLaunch(Int32)

        var errorDescription: String? {
            switch self {
            case .missingResources:
                "当前应用包不完整，无法卸载麦克风驱动。请重新安装完整应用后再试。"
            case .cleanupLaunch(let code):
                "未能启动清理程序（错误 \(code)），数据没有被删除。登录项与钥匙串账号可能已经移除。"
            }
        }
    }

    /// 按勾选项执行卸载。返回 false 表示用户取消了管理员授权，此时没有删除任何内容。
    /// 成功时启动清理脚本并退出应用，不会返回到调用方的后续界面流程。
    @MainActor
    static func perform(store: AppStore, plan: Plan, selection: Set<Component>) async throws -> Bool {
        // 1. 驱动放在最前：唯一可能被用户取消的步骤，取消时其余内容保持原样。
        if selection.contains(.microphoneDriver), !plan.microphoneDrivers.isEmpty {
            guard let resources = Bundle.main.resourceURL else { throw Failure.missingResources }
            MicrophoneSharing.shared.stop()
            do {
                let completed = try await Task.detached {
                    try MicrophoneDriverManager.run(.uninstall, resources: resources, restartAudio: true)
                }.value
                guard completed else {
                    MicrophoneSharing.shared.ensureStarted(engine: store.engine)
                    return false
                }
            } catch {
                MicrophoneSharing.shared.ensureStarted(engine: store.engine)
                throw error
            }
        }

        // 2. 以下步骤尽力而为：单项失败不应阻止删除数据。
        if selection.contains(.loginItem), SMAppService.mainApp.status != .notRegistered {
            try? await SMAppService.mainApp.unregister()
        }
        if selection.contains(.keychain) { deleteKeychainItems(services: plan.keychainServices) }
        if selection.contains(.privacy) { await resetPrivacyPermissions() }

        // 3. 最后启动清理脚本并立即退出；脚本等本进程结束后删除所选内容。
        try launchCleanup(plan, selection: selection)
        NSApp.terminate(nil)
        return true
    }

    /// 删除各服务名下的全部 generic password。文件钥匙串的 SecItemDelete
    /// 可能一次只删一条，循环到没有匹配项为止。
    static func deleteKeychainItems(services: [String]) {
        for service in services {
            let query: [String: Any] = [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service,
            ]
            for _ in 0..<256 where SecItemDelete(query as CFDictionary) == errSecSuccess {}
        }
    }

    /// 清除辅助功能、屏幕录制、输入监控等系统权限记录，重装后会重新请求授权。
    private static func resetPrivacyPermissions() async {
        await Task.detached {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/usr/bin/tccutil")
            process.arguments = ["reset", "All", bundleIdentifier]
            process.standardOutput = FileHandle.nullDevice
            process.standardError = FileHandle.nullDevice
            guard (try? process.run()) != nil else { return }
            process.waitUntilExit()
        }.value
    }

    /// 清理脚本参数：`$1` 应用 PID、`$2` 应用本体路径、`$3` 临时目录（均可为空），其余为待删除路径。
    /// 偏好设置域通过环境变量 `DESKFUSE_PREFERENCE_DOMAINS` 传入。路径只作为参数传递，从不拼进脚本。
    static let cleanupScript = #"""
    set -u
    pid=$1; bundle=$2; tmp=$3; shift 3
    # 先记下应用的子进程（输入服务、OCR helper 等），应用退出后它们会被 launchd 接管。
    children=$(/usr/bin/pgrep -P "$pid" 2>/dev/null || true)
    i=0
    while /bin/kill -0 "$pid" 2>/dev/null; do
      if [ "$i" -ge 300 ]; then /bin/kill -9 "$pid" 2>/dev/null; break; fi
      /bin/sleep 0.1
      i=$((i + 1))
    done
    for child in $children; do
      [ "$child" = "$$" ] && continue
      /bin/kill "$child" 2>/dev/null
    done
    for domain in ${DESKFUSE_PREFERENCE_DOMAINS:-}; do
      /usr/bin/defaults delete "$domain" >/dev/null 2>&1
    done
    for target in "$@"; do
      /bin/rm -rf -- "$target"
    done
    if [ -n "$tmp" ]; then
      /usr/bin/find "$tmp" -maxdepth 1 -name 'deskfuse-*' -exec /bin/rm -rf -- {} + 2>/dev/null
    fi
    if [ -n "$bundle" ] && [ -d "$bundle" ]; then
      /bin/mkdir -p "$HOME/.Trash"
      name=$(/usr/bin/basename "$bundle" .app)
      dest="$HOME/.Trash/$name.app"
      [ -e "$dest" ] && dest="$HOME/.Trash/$name $(/bin/date +%Y%m%d-%H%M%S).app"
      /bin/mv -- "$bundle" "$dest" 2>/dev/null
    fi
    exit 0
    """#

    static func cleanupArguments(_ plan: Plan, selection: Set<Component>, pid: pid_t) -> [String] {
        let bundle = selection.contains(.application) ? plan.appBundle?.path ?? "" : ""
        let temporary = selection.contains(.caches) ? plan.temporaryDirectory.path : ""
        return ["/bin/sh", "-c", cleanupScript, "deskfuse-uninstall", String(pid), bundle, temporary]
            + plan.paths(for: selection).map(\.path)
    }

    static func cleanupEnvironment(_ plan: Plan, selection: Set<Component>) -> [String: String] {
        var environment = ProcessInfo.processInfo.environment
        environment["DESKFUSE_PREFERENCE_DOMAINS"] = selection.contains(.preferences)
            ? plan.preferenceDomains.joined(separator: " ") : ""
        return environment
    }

    /// 用独立会话启动清理脚本，应用退出后脚本继续运行；标准输入输出全部指向 /dev/null。
    private static func launchCleanup(_ plan: Plan, selection: Set<Component>) throws {
        let arguments = cleanupArguments(plan, selection: selection, pid: getpid())
        let environment = cleanupEnvironment(plan, selection: selection)

        var attributes: posix_spawnattr_t?
        posix_spawnattr_init(&attributes)
        defer { posix_spawnattr_destroy(&attributes) }
        posix_spawnattr_setflags(&attributes, Int16(POSIX_SPAWN_SETSID))

        var actions: posix_spawn_file_actions_t?
        posix_spawn_file_actions_init(&actions)
        defer { posix_spawn_file_actions_destroy(&actions) }
        posix_spawn_file_actions_addopen(&actions, 0, "/dev/null", O_RDONLY, 0)
        posix_spawn_file_actions_addopen(&actions, 1, "/dev/null", O_WRONLY, 0)
        posix_spawn_file_actions_addopen(&actions, 2, "/dev/null", O_WRONLY, 0)

        let argv = arguments.map { strdup($0) } + [nil]
        let envp = environment.map { strdup("\($0.key)=\($0.value)") } + [nil]
        defer { (argv + envp).forEach { free($0) } }

        var child: pid_t = 0
        let code = posix_spawn(&child, "/bin/sh", &actions, &attributes, argv, envp)
        guard code == 0 else { throw Failure.cleanupLaunch(code) }
    }
}
