import SwiftUI
import AppKit
import UniDeskEngine

@MainActor final class PairingWindowController: NSObject, NSWindowDelegate {
    private unowned let store: AppStore
    private var panel: NSPanel?
    private var visibleIDs: Set<String> = []
    init(store: AppStore) { self.store = store }
    func reconcile(_ requests: [Pairing]) {
        let ids = Set(requests.filter(\.incoming).map(\.id))
        guard !ids.isEmpty else { panel?.orderOut(nil); visibleIDs = []; return }
        if panel == nil {
            let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 420, height: 290), styleMask: [.titled, .closable, .utilityWindow], backing: .buffered, defer: false)
            panel.title = "设备配对"; panel.isReleasedWhenClosed = false; panel.hidesOnDeactivate = false; panel.level = .floating
            panel.contentView = NSHostingView(rootView: PairingWindowView(store: store)); panel.delegate = self; panel.center(); self.panel = panel
        }
        if ids != visibleIDs { panel?.orderFrontRegardless(); visibleIDs = ids }
    }
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        for id in visibleIDs where !store.pairingDecisions.contains(id) { store.send(.reject(requestId: id), pairing: id) }
        sender.orderOut(nil); return false
    }
}
struct PairingWindowView: View {
    @Bindable var store: AppStore
    var body: some View { ScrollView { VStack(spacing: 12) { ForEach(store.snapshot?.pairing.filter(\.incoming) ?? [], id: \.id) { request in PairingRow(store: store, request: request) }; if let error = store.error { Text(error).foregroundStyle(.orange) } }.padding(16) } }
}
