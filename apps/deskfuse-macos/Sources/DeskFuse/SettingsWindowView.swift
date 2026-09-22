import SwiftUI

/// Snapshot observation stays inside this scene; presenting settings never
/// changes the selected workspace page or its clipboard selection.
struct SettingsWindowView: View {
    let store: AppStore
    @Environment(\.openSettings) private var openSettings
    @SceneStorage("settings.selectedTab") private var selectedTab = "general"

    var body: some View {
        VStack(spacing: 0) {
            ErrorNoticeView(notice: store.errorNotice)
            SettingsView(store: store, presentation: SettingsPresentation(store.snapshot)).equatable()
        }
        .modifier(WorkspaceLoadingOverlay(isPresented: store.isEngineLoading))
        .tint(.blue)
        .onAppear {
            store.showSettingsWindow = { openSettings() }
            if let tab = store.pendingSettingsTab {
                selectedTab = tab
                store.pendingSettingsTab = nil
            }
        }
    }
}
