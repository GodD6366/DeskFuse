import SwiftUI
import AppKit
import Observation
import DeskFuseRemoteApplication

enum RemoteRdpAppPresentation {
    static func windowsBasename(_ path: String) -> String {
        path.split(separator: "\\", omittingEmptySubsequences: true).last.map(String.init) ?? path
    }

    static func title(name: String, displayName: String, path: String) -> String {
        let display = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        let basename = windowsBasename(path)
        let displayProgram = normalizedProgramName(display)
        let pathProgram = normalizedProgramName(basename)
        let friendlyNames = [
            "chrome": "Google Chrome",
            "msedge": "Microsoft Edge",
            "notepad": "记事本",
            "iexplore": "Internet Explorer",
            "mstsc": "远程桌面连接",
        ]

        if let friendly = friendlyNames[displayProgram] {
            return friendly
        }
        if !display.isEmpty {
            let matchesPathProgram = display.caseInsensitiveCompare(basename) == .orderedSame
                || displayProgram == pathProgram
            if matchesPathProgram, let friendly = friendlyNames[pathProgram] {
                return friendly
            }
            return display
        }
        if let friendly = friendlyNames[pathProgram] {
            return friendly
        }
        return name
    }

    static func isSystemOrInternal(displayName: String, path: String) -> Bool {
        let components = path.split(separator: "\\", omittingEmptySubsequences: true)
            .map { $0.lowercased() }
        let isWindowsSystemApp = components.indices.contains { index in
            index + 1 < components.count
                && components[index] == "windows"
                && components[index + 1] == "systemapps"
        }
        let displayBasename = windowsBasename(displayName)
        let executable = windowsBasename(path)
        let isDiagnostic = [displayBasename, executable].contains {
            $0.caseInsensitiveCompare("IEDIAGCMD.EXE") == .orderedSame
        }
        return isWindowsSystemApp || isDiagnostic
    }

    private static func normalizedProgramName(_ value: String) -> String {
        let lowercased = value.lowercased()
        return lowercased.hasSuffix(".exe") ? String(lowercased.dropLast(4)) : lowercased
    }
}

/// Windows RemoteApp picker. The Windows catalog is intentionally separate from
/// the Mac media catalog: launching an entry starts a native RDP window rather
/// than asking the Windows host to capture a desktop surface.
@available(macOS 14.2, *)
struct RemoteRdpPicker: View {
    private enum AppScope: String, CaseIterable {
        case all = "全部"
        case favorites = "收藏"
    }

    /// 登录方式：使用账号库中的账号，或临时输入一套新账号。
    private enum AccountChoice: Hashable {
        case saved(UUID)
        case manual
    }

    @Bindable var session: RemoteRdpSession
    var embedded = false
    var launch: (RemoteRdpApp, String, String) -> Void
    private let accounts = RemoteRDPAccountStore.shared
    @State private var accountChoice = AccountChoice.manual
    @State private var username = ""
    @State private var password = ""
    @State private var saveManualAccount = true
    @State private var manualAccountName = ""
    @State private var managingAccounts = false
    @State private var credentialsMessage: String?
    @State private var selected: RemoteRdpApp?
    @State private var searchText = ""
    @State private var appScope = AppScope.all
    @State private var favoriteAppIDs = Set<String>()
    @State private var systemAppsAreExpanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // 嵌入设备页时外层已有设备名与刷新按钮，不再重复标题。
            if !embedded {
                HStack(spacing: 10) {
                    Image(systemName: "windowslogo")
                        .font(DFType.pageTitle)
                        .dfSecondaryText()
                    Text("Windows 远程应用").font(DFType.title)
                    DFBadge()
                    Spacer()
                    if session.phase == .connecting { ProgressView() }
                    Button("刷新") { session.refresh() }
                        .disabled(session.phase == .connecting)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Divider().padding(.top, 14)
            }
            content
        }
        .padding(.horizontal, embedded ? 20 : 24)
        .padding(.vertical, embedded ? 16 : 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .task {
            favoriteAppIDs = Set(UserDefaults.standard.stringArray(forKey: favoritesKey) ?? [])
            accountChoice = defaultAccountChoice
            if session.phase == .idle { await session.connect() }
        }
        .onChange(of: accounts.accounts) { _, current in
            // 账号在管理面板中被删除后回落到默认选择，避免指向不存在的账号。
            if case .saved(let id) = accountChoice, !current.contains(where: { $0.id == id }) {
                accountChoice = defaultAccountChoice
            }
        }
        .sheet(isPresented: $managingAccounts) { RemoteRDPAccountManager() }
    }

    @ViewBuilder private var content: some View {
        if case .failed(let message) = session.phase {
            statusView("无法读取 Windows 应用", systemImage: "exclamationmark.triangle", message: message)
        } else if let reason = session.unavailableReason {
            statusView("Windows 不支持 RemoteApp", systemImage: "network.slash", message: reason)
        } else if session.phase == .idle || session.phase == .connecting {
            VStack(spacing: 12) {
                ProgressView()
                Text("正在读取 Windows 应用…").dfSecondaryText()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if session.apps.isEmpty {
            statusView("没有可用的远程应用", systemImage: "square.grid.2x2",
                message: "Windows 尚未提供可用应用。新版服务会自动发现应用并在首次打开时注册；旧版服务仍需手动注册。")
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    toolbar
                    if accountChoice == .manual { manualAccountCard }
                    if let credentialsMessage {
                        Label(credentialsMessage, systemImage: "exclamationmark.triangle")
                            .font(DFType.detail)
                            .foregroundStyle(DFPalette.warning)
                    }
                    if visibleApps.isEmpty {
                        if appScope == .favorites, searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            ContentUnavailableView("还没有收藏的应用", systemImage: "star",
                                description: Text("点按应用卡片右上角的星标即可收藏。"))
                                .frame(maxWidth: .infinity, minHeight: 180)
                        } else {
                            ContentUnavailableView.search(text: searchText)
                                .frame(maxWidth: .infinity, minHeight: 180)
                        }
                    } else {
                        if !regularApps.isEmpty {
                            appGrid(regularApps)
                        }
                        if !systemApps.isEmpty {
                            Button {
                                systemAppsAreExpanded.toggle()
                            } label: {
                                HStack(spacing: 6) {
                                    Image(systemName: systemSectionIsExpanded ? "chevron.down" : "chevron.right")
                                    Text("系统与内部组件")
                                    Text("\(systemApps.count)")
                                        .dfSecondaryText()
                                }
                                .font(DFType.detail.weight(.medium))
                            }
                            .buttonStyle(.plain)
                            if systemSectionIsExpanded {
                                appGrid(systemApps)
                            }
                        }
                    }
                    Text("画面与输入经 DeskFuse 加密隧道转发，Windows 只需开启远程桌面。")
                        .font(DFType.caption)
                        .dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, embedded ? 4 : 16)
                .padding(.bottom, 8)
            }
        }
    }

    /// 账号、筛选与搜索合并为一行，应用网格是页面主体。
    private var toolbar: some View {
        HStack(spacing: 10) {
            accountMenu
            Spacer(minLength: 12)
            Picker("应用范围", selection: $appScope) {
                ForEach(AppScope.allCases, id: \.self) { scope in
                    Text(scope.rawValue).tag(scope)
                }
            }
            .labelsHidden()
            .pickerStyle(.segmented)
            .fixedSize()
            TextField("搜索应用", text: $searchText)
                .textFieldStyle(.roundedBorder)
                .frame(maxWidth: 200)
        }
    }

    private var accountMenu: some View {
        Menu {
            ForEach(accounts.accounts) { account in
                Button {
                    accountChoice = .saved(account.id)
                } label: {
                    if accountChoice == .saved(account.id) {
                        Label(account.title, systemImage: "checkmark")
                    } else {
                        Text(account.title)
                    }
                }
            }
            if !accounts.accounts.isEmpty { Divider() }
            Button("使用其他账号…", systemImage: "person.badge.plus") { accountChoice = .manual }
            Button("管理账号…", systemImage: "person.2") { managingAccounts = true }
        } label: {
            Label(accountMenuTitle, systemImage: "person.crop.circle")
                .font(DFType.bodyMedium)
                .lineLimit(1)
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .help("选择用于登录 Windows 的账号；密码仅保存在本机 macOS 钥匙串")
        .accessibilityLabel("登录账号：\(accountMenuTitle)")
        .onChange(of: accountChoice) { _, _ in credentialsMessage = nil }
    }

    private var accountMenuTitle: String {
        if case .saved(let id) = accountChoice, let account = accounts.accounts.first(where: { $0.id == id }) {
            return account.title
        }
        return "使用其他账号"
    }

    /// 只有选择“使用其他账号”时出现；用户名与密码并排，保存选项紧随其后。
    private var manualAccountCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                TextField("Windows 用户名或 Microsoft 邮箱", text: $username)
                    .textFieldStyle(.roundedBorder)
                SecureField("账户密码（不是 PIN）", text: $password)
                    .textFieldStyle(.roundedBorder)
            }
            HStack(spacing: 10) {
                Toggle("保存为常用账号", isOn: $saveManualAccount)
                    .fixedSize()
                if saveManualAccount {
                    TextField("备注名（可选）", text: $manualAccountName)
                        .textFieldStyle(.roundedBorder)
                        .frame(maxWidth: 200)
                }
                Spacer(minLength: 0)
            }
            Text(credentialsAreReady ? "点选下方应用即可登录。" : "填写后点选下方应用即可登录。")
                .font(DFType.caption)
                .dfSecondaryText()
        }
        .padding(14)
        .dfCard()
    }

    private func appGrid(_ apps: [RemoteRdpApp]) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 112), spacing: 12)], spacing: 12) {
            ForEach(apps, id: \.name) { app in
                appTile(app)
            }
        }
    }

    private func appTile(_ app: RemoteRdpApp) -> some View {
        ZStack(alignment: .topTrailing) {
            appButton(app)
            Button {
                toggleFavorite(app.name)
            } label: {
                Image(systemName: favoriteAppIDs.contains(app.name) ? "star.fill" : "star")
                    .foregroundStyle(favoriteAppIDs.contains(app.name) ? DFPalette.accent : .secondary)
                    .padding(8)
            }
            .buttonStyle(.plain)
            .help(favoriteAppIDs.contains(app.name) ? "取消收藏" : "收藏")
            .accessibilityLabel("\(favoriteAppIDs.contains(app.name) ? "取消收藏" : "收藏") \(RemoteRdpAppPresentation.title(name: app.name, displayName: app.displayName, path: app.path))")
        }
    }

    private func appButton(_ app: RemoteRdpApp) -> some View {
        Button {
            open(app)
        } label: {
            VStack(spacing: 8) {
                ZStack {
                    RoundedRectangle(cornerRadius: DFRadius.control)
                        .fill(DFPalette.accent.opacity(0.12))
                    Image(systemName: "macwindow")
                        .font(DFType.icon(.large, weight: .medium))
                        .foregroundStyle(DFPalette.accent)
                }
                .frame(width: 42, height: 42)
                .accessibilityHidden(true)
                Text(RemoteRdpAppPresentation.title(name: app.name, displayName: app.displayName, path: app.path))
                    .lineLimit(2)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity, minHeight: 86)
        }
        .buttonStyle(.dfSecondary)
        .disabled(!credentialsAreReady)
        .help(appHelp(for: app))
    }

    private var credentialsAreReady: Bool {
        switch accountChoice {
        case .saved(let id): accounts.accounts.contains { $0.id == id }
        case .manual: !username.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !password.isEmpty
        }
    }

    /// 优先使用这台设备上次登录的账号，其次账号库第一个，没有账号时进入手动输入。
    private var defaultAccountChoice: AccountChoice {
        if let id = accounts.lastAccountID(peerID: session.peerID) ?? accounts.accounts.first?.id {
            return .saved(id)
        }
        return .manual
    }

    private func open(_ app: RemoteRdpApp) {
        selected = app
        switch accountChoice {
        case .saved(let id):
            guard let account = accounts.accounts.first(where: { $0.id == id }),
                  let password = accounts.password(for: account) else {
                credentialsMessage = "钥匙串中找不到该账号的密码，请在“管理账号”中重新填写。"
                return
            }
            credentialsMessage = nil
            accounts.markUsed(id, peerID: session.peerID)
            launch(app, account.username, password)
        case .manual:
            let username = username.trimmingCharacters(in: .whitespacesAndNewlines)
            let password = password
            credentialsMessage = nil
            if saveManualAccount {
                // 保存失败不阻止本次登录，只提示下次需要重新输入。
                do {
                    let account = try accounts.save(id: nil, name: manualAccountName,
                                                    username: username, password: password)
                    accounts.markUsed(account.id, peerID: session.peerID)
                    accountChoice = .saved(account.id)
                    self.password = ""
                    manualAccountName = ""
                } catch {
                    credentialsMessage = error.userFacingMessage
                }
            }
            launch(app, username, password)
        }
    }

    private var visibleApps: [RemoteRdpApp] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        return session.apps.filter { app in
            let isInScope = appScope == .all || favoriteAppIDs.contains(app.name)
            guard isInScope else { return false }
            guard !query.isEmpty else { return true }
            return RemoteRdpAppPresentation.title(name: app.name, displayName: app.displayName, path: app.path)
                .localizedCaseInsensitiveContains(query)
                || app.name.localizedCaseInsensitiveContains(query)
                || app.path.localizedCaseInsensitiveContains(query)
        }
    }

    private var regularApps: [RemoteRdpApp] {
        visibleApps.filter { !RemoteRdpAppPresentation.isSystemOrInternal(displayName: $0.displayName, path: $0.path) }
    }

    private var systemApps: [RemoteRdpApp] {
        visibleApps.filter { RemoteRdpAppPresentation.isSystemOrInternal(displayName: $0.displayName, path: $0.path) }
    }

    private var systemSectionIsExpanded: Bool {
        systemAppsAreExpanded || !searchText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var favoritesKey: String {
        "remote-rdp-favorites.\(session.peerID)"
    }

    private func toggleFavorite(_ appID: String) {
        if !favoriteAppIDs.insert(appID).inserted {
            favoriteAppIDs.remove(appID)
        }
        UserDefaults.standard.set(favoriteAppIDs.sorted(), forKey: favoritesKey)
    }

    private func appHelp(for app: RemoteRdpApp) -> String {
        [app.name, app.path].filter { !$0.isEmpty }.joined(separator: "\n")
    }

    private func statusView(_ title: String, systemImage: String, message: String) -> some View {
        ContentUnavailableView(title, systemImage: systemImage, description: Text(message))
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

@available(macOS 14.2, *)
@MainActor final class RemoteRdpPickerWindow: NSWindowController, NSWindowDelegate {
    let session: RemoteRdpSession
    var launch: ((RemoteRdpApp, String, String) -> Void)?

    init(session: RemoteRdpSession, peerName: String) {
        self.session = session
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 720, height: 500),
            styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = "Windows 远程应用 — \(peerName)"
        window.contentMinSize = NSSize(width: 620, height: 420)
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
    let host: RemoteRdpHostConnection
    var onClose: (() -> Void)?
    init(host: RemoteRdpHostConnection, title: String) {
        self.host = host
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1024, height: 720),
            styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = title
        window.acceptsMouseMovedEvents = true
        window.isReleasedWhenClosed = false
        window.contentMinSize = NSSize(width: 480, height: 320)
        super.init(window: window)
        window.delegate = self
        // 画布没有固有尺寸；默认 sizingOptions 会把窗口收缩到提示文字大小。
        let content = NSHostingController(rootView: RemoteRdpViewer(host: host))
        content.sizingOptions = []
        window.contentViewController = content
        window.setContentSize(NSSize(width: 1024, height: 720))
        window.center()
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    func windowWillClose(_ notification: Notification) { host.stop(); onClose?() }
}

@available(macOS 14.2, *)
private struct RemoteRdpViewer: View {
    @Bindable var host: RemoteRdpHostConnection
    var body: some View {
        ZStack {
            RemoteRdpCanvas(host: host)
            if case .failed(let message) = host.phase { notice(message) }
            else if case .failed(let message) = host.bridgeState { notice(message) }
            else if host.bridgeState == .disconnected { notice("RDP 已断开。关闭此窗口后从应用列表重新打开。") }
            else if host.bridgeState != .connected { ProgressView("正在连接 Windows RDP…").padding().background(.regularMaterial) }
        }
    }
    private func notice(_ text: String) -> some View {
        Text(text)
            .textSelection(.enabled)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: 520)
            .padding()
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: DFRadius.control))
            .padding(24)
    }
}

@available(macOS 14.2, *)
private struct RemoteRdpCanvas: NSViewRepresentable {
    let host: RemoteRdpHostConnection
    func makeNSView(context: Context) -> RemoteRdpCanvasView {
        let view = RemoteRdpCanvasView(); view.host = host; return view
    }
    func updateNSView(_ view: RemoteRdpCanvasView, context: Context) { view.host = host }
    static func dismantleNSView(_ view: RemoteRdpCanvasView, coordinator: ()) { view.stop() }
}

@available(macOS 14.2, *)
@MainActor private final class RemoteRdpCanvasView: NSView {
    weak var host: RemoteRdpHostConnection?
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
        for flag in buttons { host?.mouse(x: lastPoint.0, y: lastPoint.1, flags: flag) }; buttons.removeAll()
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
        guard let host, let frame = host.frame() else { return }
        guard let provider = CGDataProvider(data: frame.bytes as CFData),
              let cgImage = CGImage(width: frame.width, height: frame.height, bitsPerComponent: 8,
                bitsPerPixel: 32, bytesPerRow: frame.width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue),
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
            host?.mouse(x: lastPoint.0, y: lastPoint.1, flags: flag)
        }
    }
    private func sendMouse(_ event: NSEvent, flags: UInt16) -> Bool {
        guard let image, window?.isKeyWindow == true, host?.bridgeState == .connected else { return false }
        let point = convert(event.locationInWindow, from: nil), rect = imageRect
        guard rect.width > 0, rect.height > 0, rect.contains(point) || !buttons.isEmpty else { return false }
        lastPoint = (UInt16(max(0, min(image.size.width - 1, (point.x - rect.minX) / rect.width * image.size.width))),
                     UInt16(max(0, min(image.size.height - 1, (rect.maxY - point.y) / rect.height * image.size.height))))
        host?.mouse(x: lastPoint.0, y: lastPoint.1, flags: flags); return true
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
        host?.key(flags: (down ? 0 : 0x8000) | (scan > 0xff ? 0x100 : 0), scancode: UInt8(scan & 0xff))
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
