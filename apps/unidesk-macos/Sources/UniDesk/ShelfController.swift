import SwiftUI
import AppKit
import Observation
import UniDeskEngine

@MainActor @Observable final class ShelfController {
    unowned let store: AppStore
    var files: [URL] = []
    var expanded = false
    var preparing = false
    private var transferID: String?
    private var lease: ExportLease?
    private var panel: ShelfPanel?
    private var dragMonitor: Any?
    private var lastPoint: NSPoint?
    private var lastDirection: CGFloat = 0
    private var reversals = 0
    private var startedAt: TimeInterval = 0
    private var lastShown: TimeInterval = 0
    init(store: AppStore) {
        self.store = store
        dragMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDragged, .leftMouseUp]) { [weak self] event in
            MainActor.assumeIsolated { self?.observeDrag(event) }
        }
    }
    func append(_ urls: [URL]) {
        guard !preparing else { return }
        var next = files
        for url in urls where url.isFileURL {
            let normalized = url.standardizedFileURL
            if !next.contains(normalized) { next.append(normalized) }
        }
        guard !next.isEmpty else { show(); return }
        // Re-adding the same files (for example dropping the stack back onto the
        // shelf) must not replace the export lease or reposition the panel.
        if next == files { show(); return }
        do {
            guard let engine = store.engine else { return }
            let newLease = try engine.acquireExport(paths: next.map(\.path))
            releaseLease(); lease = newLease; files = next; show()
        } catch { store.error = error.userFacingMessage }
    }
    func remove(_ urls: [URL]) {
        guard !preparing else { return }
        let next = files.filter { !urls.contains($0) }
        do {
            let newLease = next.isEmpty ? nil : try store.engine?.acquireExport(paths: next.map(\.path))
            releaseLease(); lease = newLease; files = next
        } catch { store.error = error.userFacingMessage }
    }
    func show(near pointer: NSPoint? = nil) {
        if panel == nil {
            let panel = ShelfPanel(contentRect: NSRect(x: 0, y: 0, width: 240, height: 240), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
            panel.level = .floating; panel.isFloatingPanel = true; panel.hidesOnDeactivate = false
            panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
            panel.isMovableByWindowBackground = true; panel.isOpaque = false; panel.backgroundColor = .clear; panel.hasShadow = true
            panel.contentView = NSHostingView(rootView: ShelfWindowContent(controller: self))
            panel.isReleasedWhenClosed = false; panel.becomesKeyOnlyIfNeeded = true
            self.panel = panel
        }
        guard let panel else { return }
        // A shake always follows its triggering pointer, even for an existing panel.
        // Appending dropped files must not move the drop target out from under them.
        if pointer != nil || !panel.isVisible {
            let point = pointer ?? NSEvent.mouseLocation
            if let screen = NSScreen.screens.first(where: { $0.frame.contains(point) }) ?? NSScreen.main {
                panel.setFrame(ShelfPlacement.frame(near: point, size: preferredSize, visibleFrame: screen.visibleFrame), display: true)
            }
        }
        panel.orderFrontRegardless()
    }
    private var preferredSize: NSSize {
        expanded ? NSSize(width: 480, height: 420) : NSSize(width: 240, height: 240)
    }
    func toggleExpanded() {
        expanded.toggle(); guard let panel else { return }
        let old = panel.frame
        var frame = NSRect(x: old.minX, y: old.maxY - preferredSize.height, width: preferredSize.width, height: preferredSize.height)
        if let screen = panel.screen ?? NSScreen.main {
            frame = ShelfPlacement.constrain(frame, to: screen.visibleFrame.insetBy(dx: 12, dy: 12))
        }
        panel.setFrame(frame, display: true, animate: true)
    }
    func close() {
        if preparing, let transferID { store.send(.cancelTransfer(id: transferID)) }
        files = []; preparing = false; transferID = nil; releaseLease(); panel?.orderOut(nil)
    }
    func send(to device: Device) {
        guard !preparing, !files.isEmpty else { return }
        preparing = true
        store.send(.sendFiles(peerId: device.id, paths: files.map(\.path)), completion: { [weak self] receipt in
            guard let self else { return }; transferID = receipt.transferId
            if !preparing { if let id = receipt.transferId { store.send(.cancelTransfer(id: id)) }; return }
            reconcileTransfers(store.snapshot?.transfers ?? [])
        }, failure: { [weak self] in self?.preparing = false })
    }
    func reconcileTransfers(_ tasks: [Transfer]) {
        guard let transferID, let task = tasks.first(where: { $0.id == transferID }) else { return }
        switch task.phase {
        case .sending, .completed:
            preparing = false; self.transferID = nil; files = []; releaseLease(); panel?.orderOut(nil)
        case .failed, .cancelled: preparing = false; self.transferID = nil
        default: break
        }
    }
    private func releaseLease() { if let lease { try? store.engine?.releaseExport(id: lease.id, outcome: .cancelled) }; lease = nil }
    func stop() { if let dragMonitor { NSEvent.removeMonitor(dragMonitor) }; dragMonitor = nil; close() }
    private func observeDrag(_ event: NSEvent) {
        if event.type == .leftMouseUp { lastPoint = nil; reversals = 0; return }
        let now = event.timestamp
        guard now - lastShown > 2, !(NSPasteboard(name: .drag).readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) ?? []).isEmpty else { return }
        let point = NSEvent.mouseLocation
        if now - startedAt > 0.8 { startedAt = now; reversals = 0; lastDirection = 0 }
        if let old = lastPoint {
            let movement = point.x - old.x
            if abs(movement) > 8 {
                let direction: CGFloat = movement > 0 ? 1 : -1
                if lastDirection != 0 && direction != lastDirection { reversals += 1 }
                lastDirection = direction
            }
        }
        lastPoint = point
        if reversals >= 3 { show(near: point); lastShown = now; reversals = 0 }
    }
}
/// AppKit screen coordinates are in points, with positive y pointing upwards.
private enum ShelfPlacement {
    static func frame(near point: NSPoint, size: NSSize, visibleFrame: NSRect) -> NSRect {
        let bounds = visibleFrame.insetBy(dx: 12, dy: 12)
        let size = NSSize(width: min(size.width, bounds.width), height: min(size.height, bounds.height))
        let gap: CGFloat = 32
        // Prefer upper-right, then upper-left, lower-left and lower-right.
        let candidates = [
            NSRect(x: point.x + gap, y: point.y + gap, width: size.width, height: size.height),
            NSRect(x: point.x - gap - size.width, y: point.y + gap, width: size.width, height: size.height),
            NSRect(x: point.x - gap - size.width, y: point.y - gap - size.height, width: size.width, height: size.height),
            NSRect(x: point.x + gap, y: point.y - gap - size.height, width: size.width, height: size.height),
        ]
        if let fitting = candidates.first(where: { bounds.contains($0) }) { return fitting }

        // Near an edge, a full quadrant may not fit. Slide along that edge but
        // preserve the pointer gap wherever possible; never use a fixed corner.
        let pointerClearance = NSRect(x: point.x - gap, y: point.y - gap, width: gap * 2, height: gap * 2)
        let clamped = candidates.map { constrain($0, to: bounds) }
        if let clear = clamped.first(where: { !$0.intersects(pointerClearance) }) { return clear }
        // Only exceptionally small visible areas cannot preserve the gap.
        return clamped.max { lhs, rhs in
            distanceSquared(from: point, to: lhs) < distanceSquared(from: point, to: rhs)
        } ?? constrain(candidates[0], to: bounds)
    }

    static func constrain(_ frame: NSRect, to bounds: NSRect) -> NSRect {
        let width = min(frame.width, bounds.width)
        let height = min(frame.height, bounds.height)
        return NSRect(
            x: min(max(frame.minX, bounds.minX), bounds.maxX - width),
            y: min(max(frame.minY, bounds.minY), bounds.maxY - height),
            width: width, height: height
        )
    }

    private static func distanceSquared(from point: NSPoint, to frame: NSRect) -> CGFloat {
        let dx = max(frame.minX - point.x, 0, point.x - frame.maxX)
        let dy = max(frame.minY - point.y, 0, point.y - frame.maxY)
        return dx * dx + dy * dy
    }
}

/// AppKit owns the panel's single glass surface; SwiftUI renders its content.
private struct ShelfWindowContent: View {
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    let controller: ShelfController

    var body: some View {
        if reduceTransparency {
            ShelfView(controller: controller)
                .background(Color(nsColor: .windowBackgroundColor), in: RoundedRectangle(cornerRadius: 20))
        } else {
            ShelfNativeGlass(controller: controller)
        }
    }
}

private struct ShelfNativeGlass: NSViewRepresentable {
    let controller: ShelfController

    func makeNSView(context: Context) -> NSView {
        let content = NSHostingView(rootView: ShelfView(controller: controller))
        content.sizingOptions = []
        let background: NSView
        if #available(macOS 26.0, *) {
            let glass = NSGlassEffectView()
            // The shelf is a lightweight desktop overlay, not an opaque popover.
            glass.style = .clear
            glass.cornerRadius = 20
            if #available(macOS 27.0, *) { glass.effectIsInteractive = true }
            glass.contentView = content
            background = glass
        } else {
            let material = NSVisualEffectView()
            material.material = .hudWindow
            material.blendingMode = .behindWindow
            material.state = .active
            material.wantsLayer = true
            material.layer?.cornerRadius = 20
            material.layer?.masksToBounds = true
            material.addSubview(content)
            background = material
        }
        content.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: background.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: background.trailingAnchor),
            content.topAnchor.constraint(equalTo: background.topAnchor),
            content.bottomAnchor.constraint(equalTo: background.bottomAnchor),
        ])
        return background
    }

    func updateNSView(_ nsView: NSView, context: Context) {}
}

final class ShelfPanel: NSPanel { override var canBecomeKey: Bool { true }; override var canBecomeMain: Bool { false } }

/// The top row keeps the panel movable. The file stack below must own its own
/// drag so starting a file drag never moves the window out from under the pointer.
private struct ShelfWindowDragHandle: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { HandleView() }
    func updateNSView(_ nsView: NSView, context: Context) {}
    final class HandleView: NSView {
        override var mouseDownCanMoveWindow: Bool { true }
    }
}

struct ShelfView: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Bindable var controller: ShelfController
    var body: some View {
        VStack(spacing: 10) {
            HStack(spacing: 8) {
                Button { controller.close() } label: {
                    shelfIcon("xmark", label: "清空并关闭")
                }
                .help("清空并关闭")
                // The drag handle must not stretch the top row: without a fixed
                // height the representable inflates the row and pushes all
                // controls down, leaving a large blank strip above them.
                ShelfWindowDragHandle()
                    .frame(maxWidth: .infinity, maxHeight: 28)
                Menu {
                    let targets = controller.store.snapshot?.devices.filter { $0.trusted && $0.connected } ?? []
                    if targets.isEmpty { Text("没有可发送的在线设备") }
                    ForEach(targets, id: \.id) { device in
                        Button(device.name) { controller.send(to: device) }
                    }
                } label: {
                    shelfIcon("paperplane", label: "发送到设备")
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .frame(width: 28, height: 28)
                .disabled(controller.files.isEmpty || controller.preparing)
                .help(controller.files.isEmpty ? "先拖入文件，再选择发送设备" : "发送到设备")
                Button { controller.toggleExpanded() } label: {
                    shelfIcon(
                        controller.expanded ? "arrow.down.right.and.arrow.up.left" : "arrow.up.left.and.arrow.down.right",
                        label: controller.expanded ? "收起暂存区" : "展开暂存区"
                    )
                }
                .help(controller.expanded ? "收起" : "展开")
            }
            .buttonStyle(.plain)
            .padding(.horizontal, 12)
            .padding(.top, 12)
            if controller.preparing { ProgressView("正在准备发送…").frame(maxWidth: .infinity, maxHeight: .infinity) }
            else if controller.files.isEmpty {
                VStack(spacing: 10) { Image(systemName: "tray").font(.system(size: 40)).foregroundStyle(.secondary); Text("拖入文件").font(.headline); Text("临时放在这里").font(.caption).foregroundStyle(.secondary) }.frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if controller.expanded {
                ScrollView {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 4), spacing: 16) {
                        ForEach(Array(controller.files.reversed()), id: \.self) { url in
                            VStack(spacing: 5) {
                                NativeFileDrag(urls: [url], store: controller.store, onSuccess: { controller.remove($0) }, onClick: {}).frame(height: 60)
                                Text(url.lastPathComponent).font(.caption).lineLimit(2)
                                Button("移除", systemImage: "xmark.circle") { controller.remove([url]) }.labelStyle(.iconOnly).buttonStyle(.plain).foregroundStyle(.secondary)
                            }
                        }
                    }.padding(14)
                }
            } else {
                NativeFileDrag(urls: controller.files, store: controller.store, onSuccess: { controller.remove($0) }, onClick: { controller.toggleExpanded() })
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .transition(reduceMotion ? .opacity : .scale(scale: 0.92).combined(with: .opacity))
                Text("\(controller.files.count) 个项目 ›")
                    .font(.headline)
                    .contentTransition(reduceMotion ? .opacity : .numericText(value: Double(controller.files.count)))
                    .animation(.easeOut(duration: reduceMotion ? 0.12 : 0.22), value: controller.files.count)
                Text("拖出全部 · 点按展开").font(.caption).foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .animation(.easeInOut(duration: reduceMotion ? 0.12 : 0.22), value: controller.files.isEmpty)
        .onDrop(of: [.fileURL], isTargeted: nil) { providers in
            guard !controller.preparing else { return false }
            for provider in providers {
                _ = provider.loadObject(ofClass: NSURL.self) { object, _ in
                    guard let url = object as? URL else { return }
                    Task { @MainActor in controller.append([url]) }
                }
            }
            return true
        }
    }

    private func shelfIcon(_ name: String, label: String) -> some View {
        Label(label, systemImage: name)
            .labelStyle(.iconOnly)
            .font(.system(size: 12, weight: .medium))
            .symbolRenderingMode(.monochrome)
            .frame(width: 28, height: 28)
            .contentShape(Rectangle())
    }

}
