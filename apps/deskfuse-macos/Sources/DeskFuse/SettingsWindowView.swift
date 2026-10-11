import SwiftUI

/// Snapshot observation stays inside this scene; presenting settings never
/// changes the selected workspace page or its clipboard selection.
struct SettingsWindowView: View {
    let store: AppStore
    @Environment(\.openSettings) private var openSettings
    @SceneStorage("settings.selectedTab") private var selectedTab = "general"

    var body: some View {
        SettingsView(store: store, presentation: SettingsPresentation(store.snapshot)).equatable()
            .overlay(alignment: .bottomLeading) {
                ErrorNoticeView(notice: store.errorNotice)
                    .frame(width: 340)
                    .padding(16)
            }
        .modifier(WorkspaceLoadingOverlay(isPresented: store.isEngineLoading))
        .tint(.blue)
        .onAppear {
            store.showSettingsWindow = { openSettings() }
            applyPendingTab()
        }
        .onChange(of: store.pendingSettingsTab) { _, _ in applyPendingTab() }
    }

    /// A deep link must also work while the settings window is already open,
    /// not only on the first appearance of the scene.
    private func applyPendingTab() {
        guard let tab = store.pendingSettingsTab else { return }
        selectedTab = tab
        store.pendingSettingsTab = nil
    }
}
