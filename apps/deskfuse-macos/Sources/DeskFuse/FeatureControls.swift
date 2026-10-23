import SwiftUI
import DeskFuseEngine

extension AppStore {
    func featureEnabled(for page: Page) -> Bool? {
        guard let snapshot else { return nil }
        switch page {
        case .clipboard: return snapshot.clipboardEnabled
        case .files: return snapshot.files.enabled
        case .input: return snapshot.input.settings.featureEnabled
        case .mouse:
            let controller = MouseButtonController.shared
            return controller.hasLoadedSettings ? controller.isFeatureEnabled : nil
        case .microphone: return MicrophoneSharing.shared.snapshot?.settings.enabled
        case .remoteApplications:
            if #available(macOS 14.2, *) { return RemoteApplicationManager.shared.enabled }
            return nil
        case .devices, .settings: return nil
        }
    }

    func setFeatureEnabled(_ enabled: Bool, for page: Page, completion: @escaping () -> Void) {
        guard let snapshot else { completion(); return }
        let command: EngineCommand
        switch page {
        case .clipboard:
            command = .setClipboard(enabled: enabled)
        case .files:
            var settings = snapshot.files
            settings.enabled = enabled
            command = .setFiles(settings: settings)
        case .input:
            var settings = snapshot.input.settings
            settings.featureEnabled = enabled
            command = .setInput(settings: settings)
        case .mouse:
            MouseButtonController.shared.setFeatureEnabled(enabled)
            completion()
            return
        case .microphone:
            MicrophoneSharing.shared.setFeatureEnabled(enabled)
            completion()
            return
        case .remoteApplications:
            if #available(macOS 14.2, *) { RemoteApplicationManager.shared.enabled = enabled }
            completion()
            return
        case .devices, .settings:
            completion()
            return
        }
        send(command, applied: completion, failed: { _ in completion() })
    }
}
