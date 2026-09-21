import SwiftUI

/// Snapshot observation stays inside this scene; presenting settings never
/// changes the selected workspace page or its clipboard selection.
struct SettingsWindowView: View {
    let store: AppStore
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        Group {
            if store.isEngineLoading {
                GlobalEngineLoadingView()
            } else if store.snapshot != nil {
                SettingsView(store: store, presentation: SettingsPresentation(store.snapshot)).equatable()
            } else {
                ContentUnavailableView(
                    "无法启动",
                    systemImage: "exclamationmark.triangle",
                    description: Text(store.error ?? "引擎未就绪")
                )
            }
        }
        .tint(.blue)
        .onAppear { store.showSettingsWindow = { openSettings() } }
    }
}
