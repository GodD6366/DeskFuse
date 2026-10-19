import SwiftUI

/// Settings and the last workspace share one window and retain their view state.
struct RootView: View {
    @Bindable var store: AppStore
    @State private var historySearchRequest = 0
    @State private var deviceSearchRequest = 0
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        HStack(spacing: 0) {
            MainSidebar(store: store)
                .padding(.top, 28) // Keep navigation below the native window buttons.
                .frame(width: 212)
            workspace
        }
        .ignoresSafeArea(.container, edges: .top)
        .background(Color(nsColor: .windowBackgroundColor).ignoresSafeArea())
        .tint(DFPalette.accent)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .modifier(WorkspaceLoadingOverlay(isPresented: store.isEngineLoading))
        .focusedSceneValue(\.searchClipboard, store.selectedPage == .clipboard ? { historySearchRequest += 1 } : nil)
        .focusedSceneValue(\.searchDevices, (store.selectedPage == .devices || store.selectedPage == .remoteApplications) ? { deviceSearchRequest += 1 } : nil)
        .onAppear {
            store.showMainWindow = { openWindow(id: "main") }
        }
        .sheet(isPresented: Binding(
            get: { store.previousInstallation.isPresented },
            set: { show in if !show { store.previousInstallation.isPresented = false } }
        )) {
            PreviousInstallationPrompt(controller: store.previousInstallation).dfFloating()
        }
        .sheet(isPresented: onboardingBinding) {
            OnboardingFlow(store: store).dfFloating()
        }
    }

    private var onboardingBinding: Binding<Bool> {
        Binding(
            get: { store.onboarding.isPresented },
            set: { show in if !show { store.onboarding.complete() } }
        )
    }


    private var workspace: some View {
        SettingsPageContainer(selection: store.selectedPage == .settings ? "settings" : "workspace", pages: [
            .init(id: "workspace", content: AnyView(MainContentView(
                store: store, page: store.workspacePage,
                historySearchRequest: historySearchRequest,
                deviceSearchRequest: deviceSearchRequest,
                localDeviceRequest: store.deviceSelectionRequest
            ).ignoresSafeArea(.container, edges: .top))),
            .init(id: "settings", content: AnyView(SettingsView(
                store: store, presentation: SettingsPresentation(store.snapshot),
                destination: store.settingsDestination
            ).equatable().ignoresSafeArea(.container, edges: .top)))
        ])
        .overlay(alignment: .bottomLeading) {
            if store.selectedPage == .settings {
                ErrorNoticeView(notice: store.errorNotice)
                    .frame(width: 320).padding(14)
            }
        }
        .dfCanvas()
        .ignoresSafeArea(.container, edges: .top)
    }

}

private struct SearchClipboardKey: FocusedValueKey {
    typealias Value = () -> Void
}

private struct SearchDevicesKey: FocusedValueKey {
    typealias Value = () -> Void
}

extension FocusedValues {
    var searchClipboard: (() -> Void)? {
        get { self[SearchClipboardKey.self] }
        set { self[SearchClipboardKey.self] = newValue }
    }

    var searchDevices: (() -> Void)? {
        get { self[SearchDevicesKey.self] }
        set { self[SearchDevicesKey.self] = newValue }
    }
}

struct ClipboardCommands: Commands {
    @FocusedValue(\.searchClipboard) private var searchClipboard
    @FocusedValue(\.searchDevices) private var searchDevices

    var body: some Commands {
        CommandGroup(after: .textEditing) {
            Button("搜索剪贴板") { searchClipboard?() }
                .keyboardShortcut("f", modifiers: .command)
                .disabled(searchClipboard == nil)
            Button("搜索设备") { searchDevices?() }
                .keyboardShortcut("f", modifiers: .command)
                .disabled(searchDevices == nil)
        }
    }
}
