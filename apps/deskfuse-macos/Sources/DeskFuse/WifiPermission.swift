import AppKit
import CoreLocation

/// macOS requires the Location permission before an app may read the current
/// Wi-Fi SSID. DeskFuse only reads that name for scene selection; it never
/// starts location updates or collects positions.
@MainActor
enum WifiPermission {
    /// One manager keeps authorization queries cheap; it never starts updates.
    private static let delegate = AuthorizationDelegate()
    private static let manager: CLLocationManager = {
        let manager = CLLocationManager()
        manager.delegate = delegate
        return manager
    }()
    private static var hasRequestedAuthorization = false

    private static var authorization: CLAuthorizationStatus { manager.authorizationStatus }

    static var isGranted: Bool {
        switch authorization {
        case .authorized, .authorizedAlways, .authorizedWhenInUse: true
        default: false
        }
    }

    static var statusText: String {
        switch authorization {
        case .authorized, .authorizedAlways, .authorizedWhenInUse: "已授权"
        case .denied: "已拒绝"
        case .restricted: "受系统限制"
        default: "未授权或需要操作"
        }
    }

    /// Request only once while undetermined. Repeated clicks must still offer
    /// a way forward if the system cannot display an authorization prompt.
    static func request() {
        guard CLLocationManager.locationServicesEnabled(),
              authorization == .notDetermined,
              !hasRequestedAuthorization else {
            openSettings()
            return
        }
        hasRequestedAuthorization = true
        NSApp.activate(ignoringOtherApps: true)
        manager.requestWhenInUseAuthorization()
    }

    static func openSettings() {
        guard let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices"),
              NSWorkspace.shared.open(url) else {
            AppStore.shared.error = "无法打开定位服务设置，请在系统设置 → 隐私与安全性 → 定位服务中为 DeskFuse 授权。"
            return
        }
    }

    private final class AuthorizationDelegate: NSObject, CLLocationManagerDelegate {
        func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
            Task { @MainActor in
                AppStore.shared.permissionPrompt.reevaluateWhilePresented()
                AppStore.shared.send(.refreshPermissions)
            }
        }
    }
}
