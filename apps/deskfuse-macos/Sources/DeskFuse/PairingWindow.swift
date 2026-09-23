import SwiftUI
import AppKit
import DeskFuseEngine

@MainActor final class PairingWindowController: NSObject, NSWindowDelegate {
    private unowned let store: AppStore
    private var panel: NSPanel?
    private var visibleIDs: Set<String> = []
    init(store: AppStore) { self.store = store }
    func reconcile(_ requests: [Pairing]) {
        let ids = Set(requests.filter(\.incoming).map(\.id))
        guard !ids.isEmpty else { panel?.orderOut(nil); visibleIDs = []; return }
        if panel == nil {
            let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: 480, height: 320), styleMask: [.titled, .closable, .fullSizeContentView], backing: .buffered, defer: false)
            panel.title = "设备配对"
            panel.isReleasedWhenClosed = false
            panel.hidesOnDeactivate = false
            panel.level = .floating
            panel.titlebarAppearsTransparent = true
            panel.backgroundColor = .clear
            panel.isOpaque = false
            panel.contentView = NSHostingView(rootView: PairingWindowView(store: store))
            panel.delegate = self
            panel.center()
            self.panel = panel
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
    
    var body: some View {
        VStack(spacing: 0) {
            // Header
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("设备配对")
                            .font(.system(size: 20, weight: .bold))
                        Text("确认设备加入网络")
                            .font(.system(size: 13))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                }
            }
            .padding(.horizontal, 24)
            .padding(.top, 52)
            .padding(.bottom, 20)
            
            // Content
            ScrollView {
                VStack(spacing: 14) {
                    ForEach(store.snapshot?.pairing.filter(\.incoming) ?? [], id: \.id) { request in
                        PairingRow(store: store, request: request)
                    }
                    
                    if let error = store.error {
                        HStack(spacing: 10) {
                            Image(systemName: "exclamationmark.triangle.fill")
                                .foregroundStyle(.orange)
                            Text(error)
                                .font(.system(size: 13))
                                .foregroundStyle(.orange)
                        }
                        .padding(.horizontal, 20)
                        .padding(.vertical, 14)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(
                            RoundedRectangle(cornerRadius: 12, style: .continuous)
                                .fill(Color.orange.opacity(0.1))
                        )
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 20)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(Color(nsColor: .windowBackgroundColor))
        )
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(
                    LinearGradient(
                        colors: [
                            Color(nsColor: .controlBackgroundColor).opacity(0.5),
                            Color(nsColor: .controlBackgroundColor).opacity(0.2)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )
        )
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .shadow(color: Color.black.opacity(0.25), radius: 30, y: 10)
        .shadow(color: Color.black.opacity(0.1), radius: 10, y: 5)
        .padding(20)
    }
}
