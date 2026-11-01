import Foundation

/// Per-user switches for the Windows RDP session. Both default to on; a missing
/// key is treated as on so upgrades keep the previous behaviour.
public struct RemoteRdpPreferences: Sendable, Equatable {
    public static let audioKey = "remoteRdp.audioEnabled"
    public static let clipboardKey = "remoteRdp.clipboardEnabled"

    public var audioEnabled: Bool
    public var clipboardEnabled: Bool

    public init(audioEnabled: Bool = true, clipboardEnabled: Bool = true) {
        self.audioEnabled = audioEnabled
        self.clipboardEnabled = clipboardEnabled
    }

    public static func load(from defaults: UserDefaults = .standard) -> RemoteRdpPreferences {
        RemoteRdpPreferences(
            audioEnabled: defaults.object(forKey: audioKey) as? Bool ?? true,
            clipboardEnabled: defaults.object(forKey: clipboardKey) as? Bool ?? true)
    }

    public func save(to defaults: UserDefaults = .standard) {
        defaults.set(audioEnabled, forKey: Self.audioKey)
        defaults.set(clipboardEnabled, forKey: Self.clipboardKey)
    }
}
