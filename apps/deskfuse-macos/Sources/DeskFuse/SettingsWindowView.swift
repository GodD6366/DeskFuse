import SwiftUI

/// Snapshot observation stays inside this scene; presenting settings never
/// changes the selected workspace page or its clipboard selection.
struct SettingsWindowView: View {
    let store: AppStore
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        VStack(spacing: 0) {
            if let error = store.error ?? store.snapshot?.error {
                HStack(alignment: .top, spacing: 12) {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .textSelection(.enabled)
                    Spacer(minLength: 8)
                    if store.error != nil { Button("关闭") { store.error = nil } }
                }
                .font(.callout).padding(12)
                .background(.orange.opacity(0.1))
            }
            SettingsView(store: store, presentation: SettingsPresentation(store.snapshot)).equatable()
        }
        .tint(.blue)
        .onAppear { store.showSettingsWindow = { openSettings() } }
    }
}
