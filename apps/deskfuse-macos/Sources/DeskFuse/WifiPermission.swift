/// Compatibility entry point for callers of the original Wi-Fi permission API.
/// Launch and settings share the same observable CoreLocation manager.
@MainActor
enum WifiPermission {
    static var isGranted: Bool { WifiPermissionAccess.shared.state == .granted }
    static var statusText: String { WifiPermissionAccess.shared.state.label }
    static func request() { WifiPermissionAccess.shared.authorize() }
    static func openSettings() { WifiPermissionAccess.shared.openSettings() }
}
