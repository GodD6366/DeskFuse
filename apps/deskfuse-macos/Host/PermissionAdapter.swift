import Foundation

struct HostPermissionSnapshot {
    var accessibility: Bool
    var inputMonitoring: Bool
}

final class MacOSPermissionObserver {
    func snapshot() -> HostPermissionSnapshot {
        HostPermissionSnapshot(accessibility: false, inputMonitoring: false)
    }
}
