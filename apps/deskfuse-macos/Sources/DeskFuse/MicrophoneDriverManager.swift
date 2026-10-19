import Foundation
import MicrophoneNative

/// Privileged changes are invoked only by a confirmed action in Settings.
/// Arguments go through AppleScript's `quoted form`, never source interpolation.
enum MicrophoneDriverManager {
    enum Action: String, Identifiable, Sendable {
        case install, uninstall
        case restartAudio = "restart-audio"
        var id: String { rawValue }
        var title: String {
            switch self {
            case .install: "安装麦克风驱动"
            case .uninstall: "卸载麦克风驱动"
            case .restartAudio: "重启音频服务"
            }
        }
    }
    static let driverBundleName = String(cString: ud_mic_driver_bundle_name()) + ".driver"
    static let installedPath = "/Library/Audio/Plug-Ins/HAL/" + driverBundleName
    static var isInstalled: Bool { FileManager.default.fileExists(atPath: installedPath) }

    /// 旧安装遗留的驱动路径，名称只来自随包的 previous-installation.json；
    /// 安装或卸载 DeskFuse 驱动时由 manage-driver.sh 校验身份后移除。
    static let legacyInstalledPath: String? = {
        struct Installation: Decodable {
            struct Microphone: Decodable { let bundleName: String }
            let microphone: Microphone
        }
        guard let url = Bundle.main.url(forResource: "previous-installation", withExtension: "json",
                                        subdirectory: "Microphone"),
              let data = try? Data(contentsOf: url),
              let installation = try? JSONDecoder().decode(Installation.self, from: data),
              installation.microphone.bundleName + ".driver" != driverBundleName else { return nil }
        return "/Library/Audio/Plug-Ins/HAL/" + installation.microphone.bundleName + ".driver"
    }()
    static var isLegacyInstalled: Bool {
        legacyInstalledPath.map { FileManager.default.fileExists(atPath: $0) } ?? false
    }

    static func run(_ action: Action, resources: URL, restartAudio: Bool) throws -> Bool {
        let script = resources.appendingPathComponent("Microphone/manage-driver.sh")
        guard FileManager.default.fileExists(atPath: script.path) else {
            throw MicrophoneAudioError.message("当前包未包含麦克风驱动管理程序，请重新安装完整应用。")
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", """
        on run argv
            set commandText to "/bin/bash " & quoted form of (item 1 of argv) & " " & quoted form of (item 2 of argv) & " " & quoted form of (item 3 of argv) & " " & quoted form of (item 4 of argv)
            try
                do shell script commandText with administrator privileges
                return "completed"
            on error messageText number errorNumber
                if errorNumber is -128 then return "cancelled"
                error messageText number errorNumber
            end try
        end run
        """, script.path, action.rawValue,
        resources.appendingPathComponent("Microphone").appendingPathComponent(driverBundleName).path,
        restartAudio ? "restart-audio" : "no-restart"]
        let output = Pipe()
        process.standardOutput = output
        process.standardError = output
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        let result = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        guard process.terminationStatus == 0 else {
            throw MicrophoneAudioError.message(result.isEmpty ? "驱动操作失败，请重试。" : result)
        }
        return result != "cancelled"
    }
}
