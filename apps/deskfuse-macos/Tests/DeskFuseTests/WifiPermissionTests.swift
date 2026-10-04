import CoreLocation
import Testing
@testable import DeskFuse

@Suite @MainActor struct WifiPermissionTests {
    @Test func wifiHasItsOwnUnifiedPermissionEntry() {
        #expect(LaunchPermission.allCases.contains(.wifi))
        #expect(LaunchPermission.wifi.pane == nil)
        #expect(LaunchPermission.wifi.title == "Wi-Fi 名称（定位）")
        #expect(LaunchPermission.wifi.purpose.contains("不采集位置"))
    }

    @Test func wifiAuthorizationIsNotInferredFromAnSSID() {
        #expect(WifiPermissionState(authorization: .authorizedAlways, servicesEnabled: true) == .granted)
        #expect(WifiPermissionState(authorization: .authorizedWhenInUse, servicesEnabled: true) == .granted)
        #expect(WifiPermissionState(authorization: .notDetermined, servicesEnabled: true) == .notDetermined)
        #expect(WifiPermissionState(authorization: .denied, servicesEnabled: true) == .denied)
        #expect(WifiPermissionState(authorization: .restricted, servicesEnabled: true) == .restricted)
        #expect(WifiPermissionState(authorization: .authorizedAlways, servicesEnabled: false) == .servicesDisabled)
    }

    @Test func emergencyShortcutIsEditableAlongsideDirectionsAndLock() {
        #expect(ShortcutEditor.titles.count == 6)
        #expect(ShortcutEditor.titles.last == "紧急收回")
    }
}
