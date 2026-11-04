import AppKit
import SwiftUI

/// An installer launch must not initialize AppStore, the engine, or permissions.
@main
@MainActor
enum DeskFuseLauncher {
    static func main() {
        let updatedLaunch = ProcessInfo.processInfo.arguments.contains("--updated-launch")
        if updatedLaunch && AppInstallation.isInstalledLocation(bundleURL: Bundle.main.bundleURL) {
            // The update helper already replaced the installed bundle and ran
            // its self-test. Never send that relaunch back through the installer.
            DeskFuseApp.main()
            return
        }
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
