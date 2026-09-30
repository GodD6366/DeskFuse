import SwiftUI
import AppKit
import DeskFuseEngine

/// A nonactivating desktop panel. Engine snapshots remain the source of truth;
/// closing the main window or switching apps never ends a transfer.
@MainActor final class TransferProgressWindowController {
    private unowned let store: AppStore
    private var panel: TransferProgressPanel?
    private var host: NSHostingView<TransferProgressOverlay>?
    private var dismissed: Set<String> = []
    private var autoDismissTasks: [String: Task<Void, Never>] = [:]
    private var initialized = false
    private var stopped = false

    init(store: AppStore) { self.store = store }

    func reconcile(_ transfers: [Transfer]) {
        guard !stopped else { return }
        if !initialized {
            dismissed.formUnion(transfers.filter { [.failed, .cancelled, .completed].contains($0.phase) }.map(\.id))
            initialized = true
        }
        scheduleCancelledDismissals(transfers)
        let active = transfers.filter { $0.phase != .completed && !dismissed.contains($0.id) }
        guard !active.isEmpty else { closePanel(); return }
        if panel == nil {
            let panel = TransferProgressPanel(contentRect: .zero,
                styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
            panel.isReleasedWhenClosed = false
            panel.hidesOnDeactivate = false
            panel.isFloatingPanel = true
            panel.becomesKeyOnlyIfNeeded = true
            panel.level = .floating
            panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
            panel.isOpaque = false
            panel.backgroundColor = .clear
            panel.hasShadow = false
            let host = NSHostingView(rootView: TransferProgressOverlay(store: store, transfers: active, dismiss: { [weak self] id in
                self?.dismiss(id)
            }))
            panel.contentView = host
            self.host = host
            self.panel = panel
        }
        guard let panel else { return }
        host?.rootView = TransferProgressOverlay(store: store, transfers: active, dismiss: { [weak self] id in
                self?.dismiss(id)
            })
        let screen = (panel.isVisible ? panel.screen : NSScreen.screens.first { $0.frame.contains(NSEvent.mouseLocation) })
            ?? NSScreen.main
        guard let frame = screen?.visibleFrame else { return }
        let height = min(CGFloat(active.count) * 100 + 16, max(100, frame.height - 32))
        panel.setFrame(NSRect(x: frame.maxX - 332, y: frame.maxY - height - 16,
                              width: 316, height: height), display: true)
        if !panel.isVisible { panel.orderFrontRegardless() }
    }

    private func scheduleCancelledDismissals(_ transfers: [Transfer]) {
        let cancelled = Set(transfers.filter { $0.phase == .cancelled && !dismissed.contains($0.id) }.map(\.id))
        for id in Array(autoDismissTasks.keys) where !cancelled.contains(id) {
            autoDismissTasks.removeValue(forKey: id)?.cancel()
        }
        for id in cancelled where autoDismissTasks[id] == nil {
            // Start once on entering cancelled; snapshots must not extend the delay.
            autoDismissTasks[id] = Task { @MainActor [weak self] in
                do { try await Task.sleep(for: .seconds(3)) }
                catch { return }
                guard !Task.isCancelled, let self, !self.stopped else { return }
                self.autoDismissTasks.removeValue(forKey: id)
                guard self.store.snapshot?.transfers.contains(where: { $0.id == id && $0.phase == .cancelled }) == true else { return }
                self.dismiss(id)
            }
        }
    }

    private func dismiss(_ id: String) {
        autoDismissTasks.removeValue(forKey: id)?.cancel()
        dismissed.insert(id)
        reconcile(store.snapshot?.transfers ?? [])
    }

    private func closePanel() {
        panel?.orderOut(nil)
        panel?.contentView = nil
        panel?.close()
        host = nil
        panel = nil
    }

    func stop() {
        stopped = true
        for task in autoDismissTasks.values { task.cancel() }
        autoDismissTasks.removeAll()
        closePanel()
    }
}

private final class TransferProgressPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

struct TransferProgressOverlay: View {
    let store: AppStore
    let transfers: [Transfer]
    let dismiss: (String) -> Void

    var body: some View {
        ScrollView {
            NativeGlassGroup {
                VStack(spacing: 8) {
                    ForEach(transfers, id: \.id) { task in
                        TransferProgressPill(task: task) {
                            if task.phase == .failed || task.phase == .cancelled { dismiss(task.id) }
                            else { store.send(.cancelTransfer(id: task.id)) }
                        }
                    }
                }.padding(8)
            }
        }
    }
}

private struct TransferProgressPill: View {
    let task: Transfer
    let cancel: () -> Void

    private var fraction: Double {
        let total = task.phase == .delivering ? task.saveTotal : task.total
        let bytes = task.phase == .delivering ? task.saved : task.transferred
        guard total > 0 else { return 0 }
        return min(1.0, Double(bytes) / Double(total))
    }

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: task.outgoing ? "arrow.up.circle.fill" : "arrow.down.circle.fill")
                .font(.system(size: 16, weight: .medium))
                .foregroundStyle(task.outgoing ? Color.accentColor : Color.green)
                .symbolEffect(.pulse, isActive: task.phase != .failed && task.phase != .cancelled)

            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 4) {
                    Text(task.name)
                        .font(.system(size: 12, weight: .medium))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(maxWidth: 150, alignment: .leading)
                    Spacer(minLength: 4)
                    Text([.sending, .receiving, .delivering].contains(task.phase) ? "\(task.phaseTitle) \(Int(fraction * 100))%" : task.phaseTitle)
                        .font(.system(size: 11, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                }

                if task.phase == .queued || task.phase == .verifying {
                    ProgressView()
                        .progressViewStyle(.linear)
                } else {
                    ProgressView(value: fraction)
                        .progressViewStyle(.linear)
                        .tint(task.outgoing ? Color.accentColor : Color.green)
                        .animation(.easeInOut(duration: 0.25), value: fraction)
                }

                Text(task.phase == .queued || task.phase == .verifying || task.phase == .delivering || task.phase == .failed || task.phase == .cancelled || fraction >= 1 ? task.detail : "\(byteCount(task.transferred)) / \(byteCount(task.total))")
                    .lineLimit(1)
                    .help(task.detail)
                    .font(.system(size: 10))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
            Button(action: cancel) {
                Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .help(task.phase == .failed || task.phase == .cancelled ? "关闭提示" : "取消传输")
            .accessibilityLabel(task.phase == .failed || task.phase == .cancelled ? "关闭提示" : "取消传输")
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .frame(width: 300, height: 92)
        .modifier(NativeGlassBackdrop(cornerRadius: 14))
    }
}
