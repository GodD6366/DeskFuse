import SwiftUI
import AppKit
import Observation
import DeskFuseRemoteApplication

/// Windows RemoteApp picker. The Windows catalog is intentionally separate from
/// the Mac media catalog: launching an entry starts a native RDP window rather
/// than asking the Windows host to capture a desktop surface.
@available(macOS 14.2, *)
struct RemoteRdpPicker: View {
    @Bindable var session: RemoteRdpSession
    var launch: (RemoteRdpApp, String?, String?) -> Void
    @State private var username = ""
    @State private var password = ""
    @State private var selected: RemoteRdpApp?

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Image(systemName: "windowslogo")
                    .font(.system(size: 22))
                    .foregroundStyle(.secondary)
                Text("Windows 远程应用").font(.title2)
                Spacer()
                if session.phase == .connecting { ProgressView() }
                Button("刷新") { session.refresh() }
                    .disabled(session.phase == .connecting)
            }
            if case .failed(let message) = session.phase {
                Text(message).foregroundStyle(.red).textSelection(.enabled)
            } else if let reason = session.unavailableReason {
                ContentUnavailableView("Windows 不支持 RemoteApp", systemImage: "network.slash", description: Text(reason))
            } else if session.apps.isEmpty {
                ContentUnavailableView("没有可用的远程应用", systemImage: "square.grid.2x2", description: Text("请在 Windows 端将应用注册为 RemoteApp。"))
            } else {
                Form {
                    TextField("Windows 用户名", text: $username)
                    SecureField("Windows 密码", text: $password)
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 4), spacing: 14) {
                        ForEach(session.apps, id: \.name) { app in
                            Button {
                                selected = app
                                launch(app, username.isEmpty ? nil : username, password.isEmpty ? nil : password)
                            } label: {
                                VStack(spacing: 8) {
                                    Image(systemName: "app.fill").font(.system(size: 28)).foregroundStyle(.secondary)
                                    Text(app.displayName.isEmpty ? app.name : app.displayName).lineLimit(2)
                                }
                                .frame(maxWidth: .infinity, minHeight: 86)
                            }
                            .buttonStyle(.bordered)
                            .disabled(username.isEmpty || password.isEmpty)
                            .help(app.path)
                        }
                    }
                }
                .formStyle(.grouped)
                Text("使用 Windows 账户密码（不是 PIN）。需已启用远程桌面和相应防火墙规则；RDP 不提供 Mac 媒体画质/暂停控制。凭据仅用于此连接，不保存。")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let selected, session.launchedName == selected.name {
                Text("正在打开：\(selected.displayName.isEmpty ? selected.name : selected.displayName)")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(24)
        .frame(minWidth: 620, minHeight: 420)
        .task {
            if session.phase == .idle { await session.connect() }
        }
    }
}

@available(macOS 14.2, *)
@MainActor final class RemoteRdpPickerWindow: NSWindowController, NSWindowDelegate {
    let session: RemoteRdpSession
    var launch: ((RemoteRdpApp, String?, String?) -> Void)?

    init(session: RemoteRdpSession) {
        self.session = session
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 720, height: 500),
            styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "Windows 远程应用 — \(session.peerID)"
        window.isReleasedWhenClosed = false
        super.init(window: window)
        window.delegate = self
        window.contentViewController = NSHostingController(rootView: RemoteRdpPicker(session: session) { [weak self] app, user, password in
            self?.launch?(app, user, password)
        })
        window.center()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    func windowWillClose(_ notification: Notification) {
        launch = nil; RemoteApplicationManager.shared.closeRdpPicker(session)
    }
}

@available(macOS 14.2, *)
@MainActor final class RemoteRdpWindow: NSWindowController, NSWindowDelegate {
    let session: RemoteRdpSession
    var onClose: (() -> Void)?
    init(session: RemoteRdpSession, title: String) {
        self.session = session
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1024, height: 720),
            styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = title
        window.acceptsMouseMovedEvents = true
        window.isReleasedWhenClosed = false
        super.init(window: window)
        window.delegate = self
        window.contentViewController = NSHostingController(rootView: RemoteRdpViewer(session: session))
        window.center()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    func windowWillClose(_ notification: Notification) { session.stop(); onClose?() }
}

@available(macOS 14.2, *)
private struct RemoteRdpViewer: View {
    @Bindable var session: RemoteRdpSession
    var body: some View {
        ZStack {
            RemoteRdpCanvas(session: session)
            if case .failed(let message) = session.phase { notice(message) }
            else if case .failed(let message) = session.bridgeState { notice(message) }
            else if session.bridgeState == .disconnected { notice("RDP 已断开。关闭此窗口后从应用列表重新打开。") }
            else if session.bridgeState != .connected { ProgressView("正在连接 Windows RDP…").padding().background(.regularMaterial) }
        }
    }
    private func notice(_ text: String) -> some View {
        Text(text).textSelection(.enabled).padding().background(.regularMaterial)
    }
}

@available(macOS 14.2, *)
private struct RemoteRdpCanvas: NSViewRepresentable {
    let session: RemoteRdpSession
    func makeNSView(context: Context) -> RemoteRdpCanvasView {
        let view = RemoteRdpCanvasView(); view.session = session; return view
    }
    func updateNSView(_ view: RemoteRdpCanvasView, context: Context) { view.session = session }
    static func dismantleNSView(_ view: RemoteRdpCanvasView, coordinator: ()) { view.stop() }
}

@available(macOS 14.2, *)
@MainActor private final class RemoteRdpCanvasView: NSView {
    weak var session: RemoteRdpSession?
    private var image: NSImage?
    private var timer: Timer?
    private var pressed = Set<UInt16>()
    private var buttons = Set<UInt16>()
    private var lastPoint: (UInt16, UInt16) = (0, 0)
    private var blur: NSObjectProtocol?
    override var acceptsFirstResponder: Bool { true }
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow(); stop()
        guard let window else { return }
        blur = NotificationCenter.default.addObserver(forName: NSWindow.didResignKeyNotification, object: window, queue: .main) { [weak self] _ in
            Task { @MainActor [weak self] in self?.releaseInput() }
        }
        timer = Timer.scheduledTimer(withTimeInterval: 1.0 / 30.0, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in self?.refreshFrame() }
        }
    }
    func stop() {
        timer?.invalidate(); timer = nil; releaseInput()
        if let blur { NotificationCenter.default.removeObserver(blur); self.blur = nil }
    }
    override func resignFirstResponder() -> Bool { releaseInput(); return super.resignFirstResponder() }
    private func releaseInput() {
        for code in pressed { sendKey(code, down: false) }; pressed.removeAll()
        for flag in buttons { session?.mouse(x: lastPoint.0, y: lastPoint.1, flags: flag) }; buttons.removeAll()
    }
    private var imageRect: NSRect {
        guard let image else { return .zero }
        let scale = min(bounds.width / image.size.width, bounds.height / image.size.height)
        let size = NSSize(width: image.size.width * scale, height: image.size.height * scale)
        return NSRect(x: (bounds.width - size.width) / 2, y: (bounds.height - size.height) / 2, width: size.width, height: size.height)
    }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.black.setFill(); dirtyRect.fill()
        image?.draw(in: imageRect, from: .zero, operation: .sourceOver, fraction: 1)
    }
    private func refreshFrame() {
        guard let session, let frame = session.frame() else { return }
        guard let provider = CGDataProvider(data: frame.bytes as CFData),
              let cgImage = CGImage(width: frame.width, height: frame.height, bitsPerComponent: 8,
                bitsPerPixel: 32, bytesPerRow: frame.width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipFirst.rawValue | CGImageByteOrder32Little.rawValue),
                provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent) else { return }
        image = NSImage(cgImage: cgImage, size: NSSize(width: frame.width, height: frame.height)); needsDisplay = true
    }
    override func mouseDown(with event: NSEvent) { window?.makeFirstResponder(self); button(event, flag: 0x100, down: true) }
    override func mouseUp(with event: NSEvent) { button(event, flag: 0x100, down: false) }
    override func rightMouseDown(with event: NSEvent) { window?.makeFirstResponder(self); button(event, flag: 0x200, down: true) }
    override func rightMouseUp(with event: NSEvent) { button(event, flag: 0x200, down: false) }
    override func otherMouseDown(with event: NSEvent) { button(event, flag: 0x400, down: true) }
    override func otherMouseUp(with event: NSEvent) { button(event, flag: 0x400, down: false) }
    override func mouseMoved(with event: NSEvent) { _ = sendMouse(event, flags: 0x0800) }
    override func mouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func rightMouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func otherMouseDragged(with event: NSEvent) { mouseMoved(with: event) }
    override func scrollWheel(with event: NSEvent) {
        let delta = Int(event.scrollingDeltaY * (event.hasPreciseScrollingDeltas ? 3 : 120))
        guard delta != 0 else { return }
        _ = sendMouse(event, flags: 0x200 | (UInt16(bitPattern: Int16(clamping: delta)) & 0x1ff))
    }
    private func button(_ event: NSEvent, flag: UInt16, down: Bool) {
        if down {
            if sendMouse(event, flags: flag | 0x8000) { buttons.insert(flag) }
        } else if buttons.remove(flag) != nil {
            session?.mouse(x: lastPoint.0, y: lastPoint.1, flags: flag)
        }
    }
    private func sendMouse(_ event: NSEvent, flags: UInt16) -> Bool {
        guard let image, window?.isKeyWindow == true, session?.bridgeState == .connected else { return false }
        let point = convert(event.locationInWindow, from: nil), rect = imageRect
        guard rect.width > 0, rect.height > 0, rect.contains(point) || !buttons.isEmpty else { return false }
        lastPoint = (UInt16(max(0, min(image.size.width - 1, (point.x - rect.minX) / rect.width * image.size.width))),
                     UInt16(max(0, min(image.size.height - 1, (rect.maxY - point.y) / rect.height * image.size.height))))
        session?.mouse(x: lastPoint.0, y: lastPoint.1, flags: flags); return true
    }
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard window?.firstResponder === self else { return false }
        keyDown(with: event); return true
    }
    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 && event.modifierFlags.contains([.control, .option]) {
            releaseInput(); window?.makeFirstResponder(nil); return
        }
        guard Self.scancodes[event.keyCode] != nil else { return }
        pressed.insert(event.keyCode); sendKey(event.keyCode, down: true)
    }
    override func keyUp(with event: NSEvent) {
        if pressed.remove(event.keyCode) != nil { sendKey(event.keyCode, down: false) }
    }
    override func flagsChanged(with event: NSEvent) {
        let modifiers: [UInt16: NSEvent.ModifierFlags] = [56: .shift, 60: .shift, 59: .control, 62: .control, 58: .option, 61: .option, 55: .command, 54: .command]
        guard let flag = modifiers[event.keyCode] else { return }
        let down = event.modifierFlags.contains(flag) && !pressed.contains(event.keyCode)
        if down { pressed.insert(event.keyCode) } else { pressed.remove(event.keyCode) }
        sendKey(event.keyCode, down: down)
    }
    private func sendKey(_ code: UInt16, down: Bool) {
        guard let scan = Self.scancodes[code] else { return }
        session?.key(flags: (down ? 0 : 0x8000) | (scan > 0xff ? 0x100 : 0), scancode: UInt8(scan & 0xff))
    }
    // macOS hardware positions -> PC set-1 codes. Windows owns the keyboard
    // layout/IME; no characters are guessed from the Mac's active layout.
    private static let scancodes: [UInt16: UInt16] = [
        0:0x1e,1:0x1f,2:0x20,3:0x21,4:0x23,5:0x22,6:0x2c,7:0x2d,8:0x2e,9:0x2f,11:0x30,
        12:0x10,13:0x11,14:0x12,15:0x13,16:0x15,17:0x14,18:0x02,19:0x03,20:0x04,21:0x05,
        22:0x07,23:0x06,24:0x0d,25:0x0a,26:0x08,27:0x0c,28:0x09,29:0x0b,30:0x1b,31:0x18,
        32:0x16,33:0x1a,34:0x17,35:0x19,36:0x1c,37:0x26,38:0x24,39:0x28,40:0x25,41:0x27,
        42:0x2b,43:0x33,44:0x35,45:0x31,46:0x32,47:0x34,48:0x0f,49:0x39,50:0x29,51:0x0e,
        53:0x01,54:0x15c,55:0x15b,56:0x2a,58:0x38,59:0x1d,60:0x36,61:0x138,62:0x11d,
        96:0x3f,97:0x40,98:0x41,99:0x3d,100:0x42,101:0x43,103:0x57,109:0x44,111:0x58,
        115:0x147,116:0x149,117:0x153,118:0x3e,119:0x14f,120:0x3c,121:0x151,122:0x3b,
        123:0x14b,124:0x14d,125:0x150,126:0x148
    ]
}
