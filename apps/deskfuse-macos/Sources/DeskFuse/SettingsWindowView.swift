import SwiftUI

/// Snapshot observation stays inside this scene; presenting settings never
/// changes the selected workspace page or its clipboard selection.
struct SettingsWindowView: View {
    let store: AppStore
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        VStack(spacing: 0) {
            if let error = store.error ?? store.snapshot?.error {
                WorkspaceErrorNotice(
                    message: error,
                    dismiss: store.error != nil ? { store.error = nil } : nil
                )
            }
            SettingsView(store: store, presentation: SettingsPresentation(store.snapshot)).equatable()
        }
        .modifier(WorkspaceLoadingOverlay(isPresented: store.isEngineLoading))
        .tint(.blue)
        .onAppear { store.showSettingsWindow = { openSettings() } }
    }
}
