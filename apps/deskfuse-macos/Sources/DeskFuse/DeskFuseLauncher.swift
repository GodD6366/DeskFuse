import AppKit
import SwiftUI

/// An installer launch must not initialize AppStore, the engine, or permissions.
@main
@MainActor
enum DeskFuseLauncher {
    static func main() {
        if AppInstallation.requiresInstallation(bundleURL: Bundle.main.bundleURL) {
            let application = NSApplication.shared
            let installer = InstallationWindowController()
            application.setActivationPolicy(.regular)
            application.delegate = installer
            withExtendedLifetime(installer) { application.run() }
        } else {
            DeskFuseApp.main()
        }
    }
}
