import SwiftUI
import DeskFuseEngine

/// The feature switch lives in the window header, independently of page tasks.
struct FeatureControls: View {
    let store: AppStore
    let page: Page
    @State private var saving = false

    var body: some View {
        if let enabled = store.featureEnabled(for: page) {
            HStack(spacing: DFSpace.s) {
                if saving { ProgressView().controlSize(.small) }
                Toggle("总开关", isOn: Binding(
                    get: { enabled },
                    set: { value in
                        saving = true
                        store.setFeatureEnabled(value, for: page) { saving = false }
                    }
                ))
                .toggleStyle(.switch)
                .labelsHidden()
                .controlSize(.small)
                .disabled(saving || store.client == nil
                          || (page == .microphone && MicrophoneSharing.shared.saving)
                          || (page == .mouse && MouseButtonController.shared.saving))
                .accessibilityLabel("\(page.rawValue)总开关")
                .help(enabled ? "关闭\(page.rawValue)并停止相关后台任务" : "开启\(page.rawValue)")
            }
            .id(page)

        }
    }
}

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
