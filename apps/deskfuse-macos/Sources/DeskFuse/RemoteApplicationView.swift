import SwiftUI
import AppKit
import Observation
import DeskFuseRemoteApplication

@MainActor @Observable final class RemoteWindowChrome {
    var normalNavigation = UserDefaults.standard.object(forKey: "remoteApplication.normalNavigation") as? Bool ?? true {
        didSet {
            guard normalNavigation != oldValue else { return }
            UserDefaults.standard.set(normalNavigation, forKey: "remoteApplication.normalNavigation")
            menuPresented = false
            navigationChanged?()
        }
    }
    @ObservationIgnored var navigationChanged: (() -> Void)?
    @ObservationIgnored var metricsChanged: (() -> Void)?
    @ObservationIgnored var titlebarColorChanged: (() -> Void)?
    var titlebarColor: RemoteTitlebarColor? {
        didSet { if titlebarColor != oldValue { titlebarColorChanged?() } }
    }
    var titlebarBackground: NSColor {
        guard let color = titlebarColor else { return .windowBackgroundColor }
        return NSColor(srgbRed: color.red, green: color.green, blue: color.blue, alpha: 1) // ui-tokens: allow 组件字段名
    }
    var menuPresented = false {
        didSet { if !menuPresented { diagnosticsPresented = false } }
    }
    var controlsRevealed = false
    var diagnosticsPresented = false
    var metricsPresented = UserDefaults.standard.object(forKey: "remoteApplication.metricsPresented") as? Bool ?? true {
        didSet {
            guard metricsPresented != oldValue else { return }
            UserDefaults.standard.set(metricsPresented, forKey: "remoteApplication.metricsPresented")
            metricsChanged?()
        }
    }
    @ObservationIgnored weak var menuButton: NSButton?

    func activateMenu(at point: NSPoint, in window: NSWindow) -> Bool {
        guard normalNavigation || controlsRevealed || menuPresented, let button = menuButton,
              button.window === window,
              button.bounds.contains(button.convert(point, from: nil)) else { return false }
        button.performClick(nil)
        return true
    }

    /// Keep the local chrome out of the remote app's way until the pointer is
    /// close to the top-right affordance. This is driven by the window event
    /// stream so the hidden trigger never becomes a transparent input shield.
    func updatePointer(_ point: NSPoint, windowSize: CGSize) {
        let distanceFromTop = windowSize.height - point.y
        let distanceFromRight = windowSize.width - point.x
        controlsRevealed = distanceFromTop <= 68 && distanceFromRight <= 160
    }
}

/// Remote canvas; normal navigation lives in the native window toolbar.
@available(macOS 14.2, *)
struct RemoteApplicationView: View {
    @Bindable var session: RemoteApplicationSession
    var chooseApplication: () -> Void
    var closeWindow: () -> Void
    var minimizeWindow: () -> Void
    var zoomWindow: () -> Void
    @Bindable var chrome: RemoteWindowChrome

    var body: some View {
        ZStack {
            VideoCanvas(session: session)
                .frame(maxWidth: .infinity, maxHeight: .infinity)

            if !session.running {
                VStack(spacing: 16) {
                    if session.failure == nil {
                        ProgressView().controlSize(.regular)
                        Text("正在打开 \(session.applicationName)…")
                            .font(DFType.detail).foregroundStyle(.secondary)
                    } else {
                        Image(systemName: session.connected ? "exclamationmark.triangle" : "network.slash")
                            .font(DFType.metric.weight(.regular)).foregroundStyle(.secondary)
                        Text(session.connected ? "无法打开应用" : "连接已中断").font(DFType.section)
                        Text(session.failure ?? session.status).font(DFType.detail).foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                            .textSelection(.enabled)
                        Button("重新连接") { session.reconnectApplication() }
                            .buttonStyle(.dfPrimary)
                    }
                }
                .padding(24).frame(maxWidth: 420)
                // Keep loading and error states on the canvas, without an inset card.
            }
        }
        .frame(minWidth: 1, minHeight: 1)
        .overlay(alignment: .bottom) {
            if session.running && !session.controlling {
                Text(session.status).font(DFType.caption)
                    .padding(.horizontal, 14).padding(.vertical, 8)
                    .background(Color(nsColor: .windowBackgroundColor), in: Capsule())
                    .overlay(Capsule().strokeBorder(.primary.opacity(0.12)))
                    .shadow(color: .black.opacity(0.12), radius: 4, y: 2)
                    .padding(12).allowsHitTesting(false)
            }
        }
        .overlay(alignment: .top) {
            if !chrome.normalNavigation {
                ZStack(alignment: .top) {
                    trafficLights.frame(maxWidth: .infinity, alignment: .leading)
                    menuControls.frame(maxWidth: .infinity, alignment: .trailing)
                }
            }
        }
        .onChange(of: session.preferences) { _, _ in session.changePreferences() }
        .task {
            while !Task.isCancelled {
                if session.running, let presentation = session.plane.frames.presentation(),
                   let color = RemoteTitlebarColor.sample(buffer: presentation.frame.buffer, layout: presentation.layout) {
                    chrome.titlebarColor = color
                }
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
            }
        }
    }
    private var trafficLights: some View {
        RemoteTrafficLights(close: closeWindow, minimize: minimizeWindow, zoom: zoomWindow)
            .frame(width: 56, height: 20)
            .padding(.horizontal, 7).padding(.vertical, 4)
            .modifier(RemoteChromeSurface(cornerRadius: DFRadius.card, color: chrome.titlebarBackground))
            .padding(.leading, 7).padding(.top, 6)
    }

    private var menuControls: some View {
        HStack(spacing: 4) {
            RemoteCanvasDragHandle().frame(width: 22, height: 22).help("拖动以移动远程窗口")
            RemoteApplicationMenu(session: session, chrome: chrome,
                chooseApplication: chooseApplication, closeWindow: closeWindow,
                minimizeWindow: minimizeWindow, zoomWindow: zoomWindow)
        }
        .font(DFType.caption.weight(.medium))
        .padding(.horizontal, 5).padding(.vertical, 3)
        .modifier(RemoteChromeSurface(cornerRadius: DFRadius.card, color: chrome.titlebarBackground))
        .padding(.trailing, 7).padding(.top, 6)
        .opacity(chrome.controlsRevealed || chrome.menuPresented ? 1 : 0)
        .animation(.easeOut(duration: 0.16), value: chrome.controlsRevealed || chrome.menuPresented)
        // The window reports pointer proximity independently, so an
        // invisible trigger is unnecessary and would steal remote input.
        .allowsHitTesting(chrome.controlsRevealed || chrome.menuPresented)
    }

}

/// Local chrome reads as a solid bar over the captured video. The remote
/// status/control bar is intentionally opaque rather than glass.
struct RemoteChromeSurface: ViewModifier {
    let cornerRadius: CGFloat
    var color: NSColor = .windowBackgroundColor
    func body(content: Content) -> some View {
        content
            .background(Color(nsColor: color), in: RoundedRectangle(cornerRadius: cornerRadius))
            .overlay(RoundedRectangle(cornerRadius: cornerRadius).strokeBorder(.primary.opacity(0.12)))
    }
}

/// Inline toolbar readout; it never participates in the remote canvas layout.
@available(macOS 14.2, *)
struct RemoteMetricsReadout: View {
    let session: RemoteApplicationSession
    @State private var remote: RemoteMediaDiagnostics?

    var body: some View {
        HStack(spacing: 8) {
            value("FPS", fpsText)
            Divider().frame(height: 9)
            value("码率", bitrateText)
            Divider().frame(height: 9)
            value("延迟", latencyText)
        }
        .font(DFType.caption.weight(.medium))
        .monospacedDigit()
        .lineLimit(1)
        .fixedSize()
        .padding(.horizontal, 4)
        .help("显示帧率 / 当前目标码率 / QUIC 往返延迟")
        .task {
            while !Task.isCancelled {
                remote = session.connected ? session.plane.diagnosticSnapshot() : nil
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
            }
        }
    }

    private var fpsText: String {
        guard session.running, session.effectiveFPS > 0 else { return "—" }
        return String(format: "%.0f", session.effectiveFPS)
    }
    private var bitrateText: String {
        guard let bitrate = remote?.media.currentBitrate, bitrate > 0 else { return "—" }
        return String(format: "%.1f Mbps", Double(bitrate) / 1_000_000)
    }
    private var latencyText: String {
        guard let rtt = remote?.rttMilliseconds, rtt.isFinite else { return "—" }
        return String(format: "%.0f ms", rtt)
    }
    private func value(_ label: String, _ text: String) -> some View {
        HStack(spacing: 4) {
            Text(label).foregroundStyle(.secondary)
            Text(text)
        }
    }
}

/// Shared menu for native navigation and the optional floating chrome.
@available(macOS 14.2, *)
struct RemoteApplicationMenu: View {
    @Bindable var session: RemoteApplicationSession
    @Bindable var chrome: RemoteWindowChrome
    var chooseApplication: () -> Void
    var closeWindow: () -> Void
    var minimizeWindow: () -> Void
    var zoomWindow: () -> Void
    var inNavigation = false

    var body: some View {
        RemoteMenuButton(chrome: chrome, inNavigation: inNavigation) {
            session.canvasFocus(false)
            chrome.menuButton?.window?.makeFirstResponder(nil)
            chrome.menuPresented.toggle()
        }
        .frame(width: inNavigation ? 32 : 30, height: inNavigation ? 32 : 28)
        .background {
            if inNavigation {
                Circle()
                    .fill(.primary.opacity(chrome.menuPresented ? 0.10 : 0.04))
                    .overlay(Circle().strokeBorder(.primary.opacity(0.12)))
            }
        }
        .fixedSize()
        .help("远程应用菜单（⌃⌥⌘空格）")
        .popover(isPresented: Binding(
            get: { chrome.normalNavigation == inNavigation && chrome.menuPresented },
            set: { if chrome.normalNavigation == inNavigation { chrome.menuPresented = $0 } }
        ), arrowEdge: .top) {
            if chrome.diagnosticsPresented {
                RemoteDiagnosticsView(session: session) { chrome.diagnosticsPresented = false }
                    .frame(width: 320, height: 360)
            } else {
                localActions.padding(14).frame(width: 300)
            }
        }
    }

    private var localActions: some View {
        VStack(alignment: .leading, spacing: 10) {
            localAction("打开其他应用…", shortcut: nil, action: chooseApplication)
            localAction("释放控制", shortcut: "⌃⌥Esc") { session.canvasFocus(false) }
            localAction(chrome.metricsPresented && chrome.normalNavigation ? "隐藏性能信息" : "显示性能信息", shortcut: nil) {
                if chrome.normalNavigation {
                    chrome.metricsPresented.toggle()
                } else {
                    chrome.metricsPresented = true
                    chrome.normalNavigation = true
                }
            }
            Button("查看详细诊断…") { chrome.diagnosticsPresented = true }
                .buttonStyle(.plain).padding(.vertical, 3)
            Divider()
            Toggle("正常顶部导航栏", isOn: $chrome.normalNavigation)
                .help("关闭后使用悬浮红绿灯与靠近显示的菜单；新窗口沿用此选择")
            RemoteApplicationDisplayModeToggle()
            Divider()
            Picker("分辨率", selection: $session.preferences.resolution) {
                ForEach(ResolutionPreference.allCases, id: \.self) { Text($0.label).tag($0) }
            }
            Picker("视频编码", selection: $session.preferences.codec) {
                ForEach(VideoCodec.allCases, id: \.self) { Text($0.label).tag($0) }
            }
            Picker("刷新率", selection: $session.preferences.refresh) {
                ForEach(RefreshPreference.allCases, id: \.self) { Text($0.label).tag($0) }
            }
            Picker("显示比例", selection: $session.scale) {
                ForEach(VideoViewScale.allCases, id: \.self) { Text($0.label).tag($0) }
            }
            Divider()
            localAction("最小化窗口", shortcut: "⌃⌥⌘M", action: minimizeWindow)
            localAction("最大化 / 还原", shortcut: "⌃⌥⌘F", action: zoomWindow)
            localAction("重新连接", shortcut: nil) { session.reconnectApplication() }.disabled(session.busy)
            localAction("关闭远程窗口", shortcut: "⌃⌥⌘W", action: closeWindow)
        }.controlSize(.small)
    }
    private func localAction(_ title: String, shortcut: String?, action: @escaping () -> Void) -> some View {
        Button {
            chrome.menuPresented = false
            action()
        } label: {
            HStack {
                Text(title)
                Spacer()
                if let shortcut { Text(shortcut).foregroundStyle(.secondary) }
            }.padding(.vertical, 3).contentShape(Rectangle())
        }.buttonStyle(.plain)
    }
}

private struct RemoteMenuButton: NSViewRepresentable {
    let chrome: RemoteWindowChrome
    let inNavigation: Bool
    let action: () -> Void
    func makeCoordinator() -> Coordinator { Coordinator(action: action) }
    func makeNSView(context: Context) -> NSButton {
        let button = NSButton(image: NSImage(systemSymbolName: "ellipsis", accessibilityDescription: "远程应用菜单")!,
                              target: context.coordinator, action: #selector(Coordinator.activate))
        button.isBordered = false
        button.setAccessibilityLabel("远程应用菜单")
        if chrome.normalNavigation == inNavigation { chrome.menuButton = button }
        return button
    }
    func updateNSView(_ button: NSButton, context: Context) {
        context.coordinator.action = action
        if chrome.normalNavigation == inNavigation { chrome.menuButton = button }
    }
    @MainActor final class Coordinator: NSObject {
        var action: () -> Void
        init(action: @escaping () -> Void) { self.action = action }
        @objc func activate() { action() }
    }
}

/// Real AppKit standard buttons, with actions routed to the local canvas window.
private struct RemoteTrafficLights: NSViewRepresentable {
    var close: () -> Void
    var minimize: () -> Void
    var zoom: () -> Void
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: NSRect(x: 0, y: 0, width: 56, height: 20))
        let types: [NSWindow.ButtonType] = [.closeButton, .miniaturizeButton, .zoomButton]
        let hints = ["关闭远程窗口（⌃⌥⌘W）", "最小化（⌃⌥⌘M）", "最大化 / 还原（⌃⌥⌘F）"]
        for (index, type) in types.enumerated() {
            guard let button = NSWindow.standardWindowButton(type, for: [.titled, .closable, .miniaturizable, .resizable]) else { continue }
            button.frame = NSRect(x: 2 + index * 18, y: 3, width: 13, height: 13)
            button.tag = index; button.target = context.coordinator; button.action = #selector(Coordinator.activate(_:))
            button.toolTip = hints[index]; button.setAccessibilityLabel(hints[index])
            view.addSubview(button)
        }
        return view
    }
    func updateNSView(_ view: NSView, context: Context) { context.coordinator.actions = self }
    @MainActor final class Coordinator: NSObject {
        var actions: RemoteTrafficLights
        init(_ actions: RemoteTrafficLights) { self.actions = actions }
        @objc func activate(_ sender: NSButton) {
            sender.window?.makeFirstResponder(nil)
            switch sender.tag { case 0: actions.close(); case 1: actions.minimize(); default: actions.zoom() }
        }
    }
}

/// Only the small local handle drags A's window. Dragging the captured application's
/// titlebar continues to forward input to B rather than moving the local window.
private struct RemoteCanvasDragHandle: NSViewRepresentable {
    func makeNSView(context: Context) -> Handle { Handle() }
    func updateNSView(_ view: Handle, context: Context) {}
    final class Handle: NSView {
        override init(frame: NSRect) {
            super.init(frame: frame)
            let icon = NSImageView(image: NSImage(systemSymbolName: "arrow.up.and.down.and.arrow.left.and.right", accessibilityDescription: "移动远程窗口")!)
            icon.translatesAutoresizingMaskIntoConstraints = false
            addSubview(icon)
            NSLayoutConstraint.activate([icon.centerXAnchor.constraint(equalTo: centerXAnchor), icon.centerYAnchor.constraint(equalTo: centerYAnchor)])
        }
        required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
        override func hitTest(_ point: NSPoint) -> NSView? { bounds.contains(convert(point, from: superview)) ? self : nil }
        override func mouseDown(with event: NSEvent) {
            window?.makeFirstResponder(nil)
            window?.performDrag(with: event)
        }
    }
}
