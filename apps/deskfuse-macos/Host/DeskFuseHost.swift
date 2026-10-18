import AppKit
import Combine
import Foundation

/// macOS host assembly. Swift UI talks to this layer; Rust collaboration owns policy.
@MainActor
final class DeskFuseHost: ObservableObject {
    let clipboardCapture = MacOSClipboardCapture()
    let clipboardApply = MacOSClipboardApply()
    let fileAuthorization = MacOSFileAuthorization()
    let permissionObserver = MacOSPermissionObserver()

    func start() async throws {
        try await clipboardCapture.startMonitoring()
    }

    func stop() async {
        try? await clipboardCapture.stopMonitoring()
    }
}
