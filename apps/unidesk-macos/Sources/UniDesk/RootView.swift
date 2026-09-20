import SwiftUI

/// A single window background supports navigation and an inset content card.
struct RootView: View {
    @Bindable var store: AppStore
    @State private var visibility: NavigationSplitViewVisibility = .all
    @State private var historySearchRequest = 0
    @State private var deviceSearchRequest = 0
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings

    var body: some View {
        HStack(spacing: 0) {
            if visibility != .detailOnly {
                MainSidebar(store: store, visibility: $visibility)
                    .frame(width: 250)
            }
            workspace
        }
        .background(Color(nsColor: .windowBackgroundColor).ignoresSafeArea())
        .tint(.blue)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .focusedSceneValue(\.searchClipboard, store.selectedPage == .clipboard ? { historySearchRequest += 1 } : nil)
        .focusedSceneValue(\.searchDevices, store.selectedPage == .devices ? { deviceSearchRequest += 1 } : nil)
        .onAppear {
            store.showMainWindow = { openWindow(id: "main") }
            store.showSettingsWindow = { openSettings() }
        }
        .sheet(isPresented: permissionPromptBinding) {
            PermissionPromptView(controller: store.permissionPrompt)
        }
    }

    private var permissionPromptBinding: Binding<Bool> {
        Binding(
            get: { store.permissionPrompt.presented != nil },
            set: { show in if !show { store.permissionPrompt.presented = nil } }
        )
    }

    private var workspace: some View {
        VStack(spacing: 0) {
            if visibility == .detailOnly {
                HStack {
                    SidebarVisibilityButton(visibility: $visibility)
                    Spacer()
                }
                // Clears the transparent titlebar drag region the card now
                // extends into, keeping the button fully clickable.
                .padding(.horizontal, 20).padding(.top, 20)
            }
            MainContentView(
                store: store,
                historySearchRequest: historySearchRequest,
                deviceSearchRequest: deviceSearchRequest,
                localDeviceRequest: store.deviceSelectionRequest
            )
        }
        .modifier(WorkspaceContentSurface())
        .padding(12)
        // The window uses full-size content, so the card can rise into the
        // titlebar area and keep the same 12 pt margin top and bottom. The
        // sidebar keeps the system safe area for the traffic lights.
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
