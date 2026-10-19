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
        // Per-pill footprint: 76 pt pill + 8 pt stack spacing, matching the
        // fixed height inside TransferProgressPill.
        let height = min(CGFloat(active.count) * 84 + 16, max(100, frame.height - 32))
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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let task: Transfer
    let cancel: () -> Void

    private var fraction: Double {
        let total = task.phase == .delivering ? task.saveTotal : task.total
        let bytes = task.phase == .delivering ? task.saved : task.transferred
        guard total > 0 else { return 0 }
        return min(1.0, Double(bytes) / Double(total))
    }

    /// Direction color identifies send/receive; it leaves the neutral palette
    /// only for failure, keeping status color rare per the material rules.
    private var tint: Color {
        switch task.phase {
        case .failed: DFPalette.warning
        case .cancelled: .secondary
        default: task.outgoing ? DFPalette.accent : DFPalette.success
        }
    }

    private var active: Bool { task.phase != .failed && task.phase != .cancelled }

    private var statusTitle: String {
        [.sending, .receiving, .delivering].contains(task.phase) ? "\(task.phaseTitle) \(Int(fraction * 100))%" : task.phaseTitle
    }

    private var detailTitle: String {
        if [.sending, .receiving].contains(task.phase), fraction < 1 {
            return "\(byteCount(task.transferred)) / \(byteCount(task.total))"
        }
        return task.detail
    }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: task.outgoing ? "arrow.up" : "arrow.down")
                .font(DFType.section)
                .foregroundStyle(tint)
                .frame(width: 30, height: 30)
                .background(tint.opacity(0.14), in: Circle())
                .symbolEffect(.pulse, isActive: active && !reduceMotion)
                .accessibilityLabel(task.outgoing ? "发送" : "接收")

            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 8) {
                    Text(task.name)
                        .font(DFType.detail.weight(.semibold))
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer(minLength: 0)
                    Text(statusTitle)
                        .font(DFType.caption.weight(.medium))
                        .monospacedDigit()
                        .foregroundStyle(task.phase == .failed ? DFPalette.warning : .secondary)
                        .fixedSize()
                    Button(action: cancel) {
                        Image(systemName: "xmark")
                            .font(DFType.caption.weight(.bold))
                            .foregroundStyle(.secondary)
                            .frame(width: 20, height: 20)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help(task.phase == .failed || task.phase == .cancelled ? "关闭提示" : "取消传输")
                    .accessibilityLabel(task.phase == .failed || task.phase == .cancelled ? "关闭提示" : "取消传输")
                }

                TransferProgressBar(fraction: fraction, tint: tint, indeterminate: task.phase == .queued || task.phase == .verifying)
                    .animation(.easeInOut(duration: 0.25), value: fraction)

                Text(detailTitle)
                    .font(DFType.caption)
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .help(task.detail)
            }
            .padding(.top, 1)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        // Fixed height: the panel sizes itself from a per-pill constant, so the
        // pill must not grow or shrink with state wording.
        .frame(width: 300, height: 76)
        .modifier(NativeGlassBackdrop(cornerRadius: DFRadius.panel))
    }
}

/// A 4 pt capsule bar. The system linear ProgressView reads as a chunky gray
/// stripe on glass; the slim capsule keeps the track quiet and the tint on the
/// moving part only. Indeterminate sweeps a single segment across the track.
private struct TransferProgressBar: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let fraction: Double
    let tint: Color
    let indeterminate: Bool
    @State private var sweep: CGFloat = 0

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .leading) {
                Capsule().fill(Color.primary.opacity(0.10))
                if indeterminate {
                    Capsule()
                        .fill(tint)
                        .frame(width: geometry.size.width * 0.38)
                        .offset(x: (geometry.size.width * 1.38) * sweep - geometry.size.width * 0.38)
                } else {
                    Capsule()
                        .fill(tint)
                        .frame(width: max(4, geometry.size.width * fraction))
                }
            }
        }
        .frame(height: 4)
        .clipShape(Capsule())
        .onAppear { updateSweep() }
        .onChange(of: indeterminate) { updateSweep() }
    }

    private func updateSweep() {
        guard indeterminate, !reduceMotion else {
            var transaction = Transaction()
            transaction.disablesAnimations = true
            withTransaction(transaction) { sweep = 0 }
            return
        }
        sweep = 0
        withAnimation(.linear(duration: 1.4).repeatForever(autoreverses: false)) { sweep = 1 }
    }
}
