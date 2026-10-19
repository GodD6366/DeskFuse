import AppKit
import Observation
@preconcurrency import CoreLocation

/// Location authorization is required by macOS to reveal an associated SSID;
/// a disconnected network is not evidence that authorization was denied.
enum WifiPermissionState: Equatable {
    case granted, notDetermined, denied, restricted, servicesDisabled
    init(authorization: CLAuthorizationStatus, servicesEnabled: Bool) {
        guard servicesEnabled else { self = .servicesDisabled; return }
        switch authorization {
        case .authorizedAlways, .authorizedWhenInUse: self = .granted
        case .denied: self = .denied
        case .restricted: self = .restricted
        default: self = .notDetermined
        }
    }
    var label: String {
        switch self {
        case .granted: "已授权"
        case .notDetermined: "尚未授权"
        case .denied: "已拒绝"
        case .restricted: "受系统限制"
        case .servicesDisabled: "定位服务已关闭"
        }
    }
}

@MainActor @Observable final class WifiPermissionAccess: NSObject, CLLocationManagerDelegate {
    static let shared = WifiPermissionAccess()
    @ObservationIgnored private var hasRequestedAuthorization = false
    @ObservationIgnored private let manager = CLLocationManager()
    private(set) var state: WifiPermissionState = .notDetermined
    override init() {
        super.init()
        manager.delegate = self
        refresh()
    }
    func refresh() {
        state = WifiPermissionState(authorization: manager.authorizationStatus,
                                    servicesEnabled: CLLocationManager.locationServicesEnabled())
    }
    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        Task { @MainActor [weak self] in
            self?.refresh()
            AppStore.shared.permissions.refresh()
            AppStore.shared.send(.refreshPermissions)
        }
    }
    func authorize() {
        refresh()
        if state == .notDetermined && !hasRequestedAuthorization {
            hasRequestedAuthorization = true
            NSApp.activate(ignoringOtherApps: true)
            manager.requestWhenInUseAuthorization()
        } else {
            openSettings()
        }
        // Never startUpdatingLocation or scan nearby networks.
    }
    func openSettings() {
        guard let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_LocationServices"),
              NSWorkspace.shared.open(url) else {
            AppStore.shared.error = "无法打开定位服务设置，请在系统设置 → 隐私与安全性 → 定位服务中为 DeskFuse 授权。"
            return
        }
    }
}
