import SwiftUI
import AppKit
import Observation
import UniDeskRemoteApplication

@MainActor @Observable final class RemoteWindowChrome {
    var menuPresented = false
}

/// Borderless remote canvas with local traffic lights and a centered menu.
@available(macOS 14.2, *)
struct RemoteApplicationView: View {
    @Bindable var session: RemoteApplicationSession
    var chooseApplication: () -> Void
    var closeWindow: () -> Void
    var minimizeWindow: () -> Void
    var zoomWindow: () -> Void
    @Bindable var chrome: RemoteWindowChrome
    @State private var hovering = false

    var body: some View {
        ZStack {
            VideoCanvas(session: session)
                .frame(maxWidth: .infinity, maxHeight: .infinity)

            if !session.running {
                VStack(spacing: 16) {
                    if session.failure == nil {
                        ProgressView()
                        Text("正在打开 \(session.applicationName)…")
                    } else {
                        Image(systemName: "network.slash").font(.largeTitle)
                        Text(session.connected ? "无法打开应用" : "连接已中断").font(.headline)
                        Text(session.failure ?? session.status).font(.callout).multilineTextAlignment(.center)
                            .textSelection(.enabled)
                        Button("重新连接") { session.reconnectApplication() }
                            .buttonStyle(.borderedProminent)
                    }
                }
                .padding(24).frame(maxWidth: 420)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16))
            }
        }
        .frame(minWidth: 1, minHeight: 1)
        .onHover { hovering = $0 }
        .overlay(alignment: .bottom) {
            if session.running && !session.controlling {
                Text(session.status).font(.caption)
                    .padding(.horizontal, 14).padding(.vertical, 8)
                    .background(.regularMaterial, in: Capsule())
                    .padding(12).allowsHitTesting(false)
            }
        }
        .overlay(alignment: .topLeading) {
            RemoteTrafficLights(close: closeWindow, minimize: minimizeWindow, zoom: zoomWindow)
                .frame(width: 60, height: 16)
                .padding(.horizontal, 8).padding(.vertical, 6)
                // This must be opaque: a material would let B's screen-control
                // indicator show through the local window controls.
                .background(Color(nsColor: .windowBackgroundColor), in: Capsule())
                .padding(.leading, 6).padding(.top, 5)
        }
        .overlay(alignment: .top) {
            HStack(spacing: 4) {
                RemoteCanvasDragHandle().frame(width: 24, height: 26).help("拖动以移动远程窗口")
                Button {
                    session.canvasFocus(false)
                    chrome.menuPresented.toggle()
                } label: { Image(systemName: "ellipsis").frame(width: 24, height: 26) }
                .buttonStyle(.plain).help("远程应用菜单（⌃⌥⌘空格）")
                .popover(isPresented: $chrome.menuPresented, arrowEdge: .top) {
                    localActions.padding(14).frame(width: 300)
                }
            }
            .font(.system(size: 12)).padding(.horizontal, 6).padding(.vertical, 3)
            .background(.regularMaterial, in: Capsule()).padding(.top, 8)
            .opacity(hovering || chrome.menuPresented || session.failure != nil ? 1 : 0)
            .allowsHitTesting(hovering || chrome.menuPresented || session.failure != nil)
        }
        .onChange(of: session.preferences) { _, _ in session.changePreferences() }
    }
    private var localActions: some View {
        VStack(alignment: .leading, spacing: 10) {
            localAction("打开其他应用…", shortcut: nil, action: chooseApplication)
            localAction("释放控制", shortcut: "⌃⌥Esc") { session.canvasFocus(false) }
            Divider()
            Picker("分辨率", selection: $session.preferences.resolution) {
                ForEach(ResolutionPreference.allCases, id: \.self) { Text($0.label).tag($0) }
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

/// Real AppKit standard buttons, with actions routed to the local canvas window.
private struct RemoteTrafficLights: NSViewRepresentable {
    var close: () -> Void
    var minimize: () -> Void
    var zoom: () -> Void
    func makeCoordinator() -> Coordinator { Coordinator(self) }
    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: NSRect(x: 0, y: 0, width: 60, height: 16))
        let types: [NSWindow.ButtonType] = [.closeButton, .miniaturizeButton, .zoomButton]
        let hints = ["关闭远程窗口（⌃⌥⌘W）", "最小化（⌃⌥⌘M）", "最大化 / 还原（⌃⌥⌘F）"]
        for (index, type) in types.enumerated() {
            guard let button = NSWindow.standardWindowButton(type, for: [.titled, .closable, .miniaturizable, .resizable]) else { continue }
            button.frame = NSRect(x: index * 22, y: 1, width: 14, height: 14)
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
