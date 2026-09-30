import CoreLocation
import DeskFuseEngine

/// macOS requires the Location permission before an app may read the current
/// Wi-Fi SSID. DeskFuse only reads that name for scene selection; it never
/// starts location updates or collects positions.
@MainActor
enum WifiPermission {
    /// One manager keeps authorization queries cheap; it never starts updates.
    private static let manager = CLLocationManager()

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

    /// Not determined shows the system prompt; denied/restricted opens the
    /// Location Services pane, both through the shared FFI entry point.
    static func request() {
        requestWifiAccess()
    }
}
