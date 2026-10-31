import SwiftUI
import AppKit
import Observation
import DeskFuseEngine

@MainActor @Observable final class AppStore {
    static let shared = AppStore()
    let errorNotice = ErrorNotice()
    var snapshot: EngineSnapshot? {
        didSet {
            let message = snapshot?.error
                ?? snapshot?.clipboardError
                ?? snapshot?.input.error
                ?? snapshot?.update.error
            errorNotice.updateRuntimeError(message)
        }
    }
    var error: String? {
        didSet { errorNotice.updateLocalError(error) }
    }
    var starting = true
    /// 升级后首次启动的数据优化阶段；只影响加载遮罩文案。
    var optimizingLocalData = false
    var selfTest = false
    /// A connected client may precede its first snapshot. Do not flash a
    /// failure page during that gap, or mask a real startup failure.
    var isEngineLoading: Bool {
        !selfTest && error == nil && (starting || (client != nil && snapshot == nil))
    }
    var selectedPage: Page = .devices {
        didSet { if selectedPage != .settings { workspacePage = selectedPage } }
    }
    /// Keep the workspace mounted while settings occupies the detail area.
    private(set) var workspacePage: Page = .devices
    var settingsDestination = SettingsDestination()
    /// Local macOS chrome preference; never part of synced engine business state.
    var hideDockIcon = UserDefaults.standard.bool(forKey: AppStore.hideDockIconDefaultsKey)
    static let hideDockIconDefaultsKey = "deskfuse.hideDockIcon"
    var keepAwake = UserDefaults.standard.bool(forKey: AppStore.keepAwakeDefaultsKey) {
        didSet {
            UserDefaults.standard.set(keepAwake, forKey: AppStore.keepAwakeDefaultsKey)
            if keepAwake { caffeineController.enable() }
            else { caffeineController.disable() }
        }
    }
    static let keepAwakeDefaultsKey = "deskfuse.keepAwake"
    /// 工作流属于进阶功能：默认关闭，关闭时隐藏全部入口并停用全局工作流快捷键。
    var workflowsEnabled = UserDefaults.standard.bool(forKey: AppStore.workflowsEnabledDefaultsKey) {
        didSet {
            UserDefaults.standard.set(workflowsEnabled, forKey: AppStore.workflowsEnabledDefaultsKey)
            engine?.setWorkflowHotkeysEnabled(enabled: workflowsEnabled)
        }
    }
    static let workflowsEnabledDefaultsKey = "deskfuse.workflowsEnabled"
    private let caffeineController = CaffeineController()
    @ObservationIgnored private let displayWallpaperPublisher = DisplayWallpaperPublisher()
    @ObservationIgnored private var wallpaperPublicationTask: Task<Void, Never>?
    var localDisplayWallpapers: [String: NSImage] = [:]
    private(set) var updateCheckPending = false

    var isCheckingForUpdates: Bool { updateCheckPending || snapshot?.update.checking == true }

    func checkForUpdates() {
        guard !isCheckingForUpdates else { return }
        updateCheckPending = true
        send(.checkUpdates, applied: { [weak self] in
            self?.finishUpdateCheckFeedback()
        }, failed: { [weak self] _ in
            self?.finishUpdateCheckFeedback()
        })
    }

    private func finishUpdateCheckFeedback() {
        // Keep the affordance visibly active even when the updater rejects the
        // command immediately (for example in a manual or unavailable build).
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(1200))
            self?.updateCheckPending = false
        }
    }
    var clipboardNavigation: ClipboardNavigationRequest?
    /// One-shot request from the sidebar footer to show the local device in the
    /// devices inspector without duplicating selection state in the store.
    var deviceSelectionRequest = 0
    var pendingRequests: Set<UInt64> = []
    var pairingDecisions: Set<String> = []
    var client: EngineClient?
    // These nonblocking native calls bypass the actor queue for emergency/drag callbacks.
    var engine: Engine?
    var shelf: ShelfController?
    /// 权限状态的唯一来源；各功能按需申请。
    let permissions = PermissionCenter()
    /// 旧页面使用的访问器；实际状态仍只有 PermissionCenter 一个来源。
    var permissionPrompt: PermissionCenter { permissions }
    let onboarding = OnboardingController()
    let previousInstallation = PreviousInstallationController()
    var showMainWindow: (() -> Void)?
    var pairingWindow: PairingWindowController?
    private var transferWindow: TransferProgressWindowController?
    /// WebView 接管浮动窗口后，收起原生配对与传输进度面板。
    func concealNativeAuxiliaryWindows() {
        pairingWindow?.concealForWebView()
        transferWindow?.concealForWebView()
    }
    private var eventsTask: Task<Void, Never>?
    private enum CommandOutcome { case applied, failed(String) }
    private var earlyResults: [UInt64: CommandOutcome] = [:]
    private var appliedHandlers: [UInt64: () -> Void] = [:]
    private var errorHandlers: [UInt64: (String) -> Void] = [:]
    private var failureHandlers: [UInt64: () -> Void] = [:]
    private var requestPairings: [UInt64: String] = [:]
    /// 用户主动移除过的屏幕不再自动加回；持久保存，重启后仍尊重用户的选择。
    static let autoJoinSuppressedDefaultsKey = "deskfuse.input.autoJoinSuppressedPeers"
    private var autoJoinSuppressedPeers = Set(UserDefaults.standard.stringArray(
        forKey: AppStore.autoJoinSuppressedDefaultsKey) ?? []) {
        didSet {
            UserDefaults.standard.set(autoJoinSuppressedPeers.sorted(), forKey: AppStore.autoJoinSuppressedDefaultsKey)
        }
    }
    /// 自动加入的退避时间：设置命令可能被输入服务静默拒绝，失败回调不会触发。
    private var autoJoinRetryAt: [String: TimeInterval] = [:]
    private var quitting = false

    func start() {
        guard starting, engine == nil else { return }
        let startupBegan = ProcessInfo.processInfo.systemUptime
        func recordStartup(_ stage: String) {
            NSLog("DeskFuse startup %@ %.0f ms", stage,
                  (ProcessInfo.processInfo.systemUptime - startupBegan) * 1000)
        }
        // 根据保存的设置恢复常亮状态
        if keepAwake { caffeineController.enable() }
        do {
            let args = ProcessInfo.processInfo.arguments
            let transaction = args.firstIndex(of: "--update-self-test").flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil }
            if args.contains("--update-self-test") && transaction == nil { throw StartupError.invalidTransaction }
            selfTest = try prepareStartup(transaction: transaction)
            recordStartup("recovery_ready")
            if selfTest { starting = false; return }
        } catch { self.error = error.userFacingMessage; starting = false; return }
        Task {
            do {
                await optimizeLocalDataIfNeeded()
                recordStartup("data_ready")
                let engine = try await Task.detached { try Engine() }.value
                recordStartup("engine_ready")
                self.engine = engine
                engine.setWorkflowHotkeysEnabled(enabled: workflowsEnabled)
                MouseButtonController.shared.start(engine: engine)
                if #available(macOS 14.2, *) { RemoteApplicationManager.shared.start(engine: engine) }
                await WorkflowStore.shared.reload(engine: engine)
                DeskFuseImageAppShortcuts.updateAppShortcutParameters()
                let client = EngineClient(engine: engine)
                self.client = client
                let (stream, continuation) = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
                let relay = EventRelay(continuation)
                eventsTask = Task { [weak store = self] in
                    for await _ in stream {
                        guard !Task.isCancelled else { break }
                        for event in relay.takePendingEvents() {
                            guard !Task.isCancelled else { break }
                            store?.consume(event)
                        }
                        await Task.yield()
                    }
                }
                try await client.listen(relay)
                recordStartup("listener_ready")
                // 先取一次局域网共享状态，第一次打开文件页也能直接用已知结果渲染。
                LanWebSharing.shared.warmUp(client)
                transferWindow = TransferProgressWindowController(store: self)
                transferWindow?.reconcile(snapshot?.transfers ?? [])
                shelf = ShelfController(store: self)
                pairingWindow = PairingWindowController(store: self)
                pairingWindow?.reconcile(snapshot?.pairing ?? [])
                starting = false
                recordStartup("workspace_ready")
                MicrophoneSharing.shared.start(engine: engine)
                // Publish the local desktop preview independently of the current
                // page. A controller can then show a peer's wallpaper even when
                // that peer has never opened the keyboard-sharing page.
                publishDisplayWallpapers()
            } catch { self.error = error.userFacingMessage; starting = false }
        }
    }

    /// 引擎启动前整理本地数据库：旧版本数据库在升级后转换一次，之后只做增量回收。
    /// 优化失败不阻断启动，下次启动会重新判断。
    private func optimizeLocalDataIfNeeded() async {
        guard await Task.detached(operation: { localDataNeedsOptimization() }).value else { return }
        optimizingLocalData = true
        defer { optimizingLocalData = false }
        do {
            let result = try await Task.detached { try optimizeLocalData() }.value
            if let reason = result.deferredReason {
                NSLog("DeskFuse data optimization deferred: %@", reason)
            } else {
                NSLog("DeskFuse data optimization %@ %llu -> %llu bytes",
                      result.performed ? "performed" : "skipped", result.bytesBefore, result.bytesAfter)
            }
        } catch {
            NSLog("DeskFuse data optimization failed: %@", error.userFacingMessage)
        }
    }

    /// 已配对且在线的设备自动加入键鼠参与者：用户只需要配对一次，
    /// 不必再到键鼠页里手动添加。只在本机是主控且功能已开启时执行。
    private func autoJoinInputParticipants(_ value: EngineSnapshot) {
        let input = value.input
        for id in input.settings.participants { autoJoinRetryAt[id] = nil }
        guard input.settings.featureEnabled, input.canConfigure, input.settings.enabled, input.settings.controller else { return }
        let candidates = value.devices
            .filter { $0.id != value.localId && $0.member && $0.trusted && $0.connected && !$0.blocked }
            .map(\.id)
        let now = ProcessInfo.processInfo.systemUptime
        let additions = candidates.filter { id in
            !input.settings.participants.contains(id)
                && !autoJoinSuppressedPeers.contains(id)
                && (autoJoinRetryAt[id] ?? 0) <= now
        }
        guard !additions.isEmpty else { return }
        // 命令回执只代表主进程已转发；输入服务仍可能拒绝或未应用设置。
        // 5 秒后若快照里仍没有该参与者就重试，避免必须重启应用才能恢复。
        for id in additions { autoJoinRetryAt[id] = now + 5 }
        var settings = input.settings
        settings.participants.append(contentsOf: additions)
        send(.setInput(settings: settings))
    }

    /// 用户手动移除屏幕后，不再自动加回，直到用户再次手动添加。
    func suppressAutoJoin(_ id: String) {
        autoJoinSuppressedPeers.insert(id)
        autoJoinRetryAt[id] = nil
    }

    /// 用户手动添加屏幕后，恢复该设备的自动加入规则。
    func allowAutoJoin(_ id: String) {
        autoJoinSuppressedPeers.remove(id)
        autoJoinRetryAt[id] = nil
    }

    /// Capture local wallpaper previews for the layout UI and publish them to
    /// peers. The page refreshes its temporary images when it is opened; the
    /// launch-time call keeps remote cards populated before that page is shown.
    func refreshDisplayWallpapers() async {
        guard snapshot?.input.settings.featureEnabled == true, !Task.isCancelled,
              let result = await displayWallpaperPublisher.refresh() else { return }
        guard snapshot?.input.settings.featureEnabled == true, !Task.isCancelled else { return }
        localDisplayWallpapers = localDisplayWallpapers.filter { result.activeDisplayIDs.contains($0.key) }
        for (id, preview) in result.previews {
            localDisplayWallpapers[id] = preview.image
            self.send(.publishDisplayWallpaper(displayId: id, jpeg: preview.jpeg))
        }
    }

    func clearDisplayWallpapers() {
        localDisplayWallpapers.removeAll(keepingCapacity: false)
    }

    private func publishDisplayWallpapers() {
        wallpaperPublicationTask?.cancel()
        guard snapshot?.input.settings.featureEnabled == true else {
            wallpaperPublicationTask = nil
            clearDisplayWallpapers()
            return
        }
        wallpaperPublicationTask = Task { [weak self] in
            guard let self else { return }
            // Coalesce discovery/activation snapshots into one capture.
            do { try await Task.sleep(for: .milliseconds(300)) } catch { return }
            await refreshDisplayWallpapers()
            if selectedPage != .input { clearDisplayWallpapers() }
        }
    }

    func consume(_ event: EngineEvent) {
        switch event {
        case .workflowCatalogChanged:
            Task { await WorkflowStore.shared.refresh() }
        case .workflowRun(let run):
            WorkflowStore.shared.consume(run)
        case .snapshot(let value):
            // A replayed snapshot must not invalidate every observing page or
            // repeat transfer/pairing reconciliation. Equal revisions can still
            // carry different payloads, so retain the full equality check.
            guard value.revision != snapshot?.revision || value != snapshot else { return }
            if value.revision >= (snapshot?.revision ?? 0) {
                let connected = Set(value.devices.filter { $0.member && $0.connected }.map(\.id))
                let previousConnected = Set(snapshot?.devices.filter { $0.member && $0.connected }.map(\.id) ?? [])
                let shouldPublishWallpaper = !connected.subtracting(previousConnected).isEmpty
                    || (value.input.managed && snapshot?.input.managed != true)
                    || value.input.settings.featureEnabled != snapshot?.input.settings.featureEnabled
                snapshot = value
                ClipboardPanelController.shared.setFeatureEnabled(value.clipboardEnabled)
                ClipboardOCRIndexer.shared.setFeatureEnabled(value.clipboardEnabled)
                if shouldPublishWallpaper { publishDisplayWallpapers() }
                if #available(macOS 14.2, *), !value.input.canOperateRemoteApplications {
                    RemoteApplicationManager.shared.reconcileViewerRole()
                }
                pairingDecisions.formIntersection(Set(value.pairing.map(\.id)))
                autoJoinInputParticipants(value)
                shelf?.reconcileTransfers(value.transfers)
                transferWindow?.reconcile(value.transfers)
                pairingWindow?.reconcile(value.pairing)
            }
        case .clipboardLibraryChanged(let revision):
            ClipboardLibraryStore.shared.libraryChanged(revision)
        case .commandApplied(let id):
            if pendingRequests.remove(id) == nil { earlyResults[id] = .applied }
            failureHandlers.removeValue(forKey: id)
            errorHandlers.removeValue(forKey: id)
            requestPairings.removeValue(forKey: id)
            appliedHandlers.removeValue(forKey: id)?()
        case .commandFailed(let id, let message):
            if pendingRequests.remove(id) == nil { earlyResults[id] = .failed(message) }; error = message
            appliedHandlers.removeValue(forKey: id)
            errorHandlers.removeValue(forKey: id)?(message)
            failureHandlers.removeValue(forKey: id)?()
            if let request = requestPairings.removeValue(forKey: id) { pairingDecisions.remove(request) }
        case .pairingFailed(let id, let message):
            pairingDecisions.remove(id); error = message
        case .pasteReady(let pid, let count):
            Task {
                do {
                    guard pid > 0 else { throw ClipboardPanelFailure.message("无法确认原粘贴目标；文件已就绪，可手动粘贴。") }
                    for _ in 0..<100 {
                        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {
                            throw ClipboardPanelFailure.message("目标应用已切换；文件已就绪，可手动粘贴。")
                        }
                        if NSEvent.modifierFlags.intersection([.command, .option, .control, .shift]).isEmpty {
                            try clipboardPasteIfCurrent(targetPid: pid, expectedChangeCount: count)
                            return
                        }
                        try await Task.sleep(for: .milliseconds(20))
                    }
                    throw ClipboardPanelFailure.message("修饰键尚未释放；文件已就绪，可手动粘贴。")
                } catch { self.error = error.userFacingMessage }
            }
        case .stageFiles(let paths): shelf?.append(paths.map { URL(fileURLWithPath: $0) })
        case .quit: NSApp.terminate(nil)
        }
    }
    /// `completion` is a submission receipt; `applied` confirms worker completion.
    func send(_ command: EngineCommand, pairing: String? = nil,
              completion: ((CommandReceipt) -> Void)? = nil, failure: (() -> Void)? = nil,
              applied: (() -> Void)? = nil, failed: ((String) -> Void)? = nil) {
        guard let client else {
            let message = "引擎尚未就绪"
            // A menu command can arrive before the window is ready. Keep
            // normal initialization in loading state and preserve real errors.
            if !starting && error == nil { error = message }
            failure?()
            failed?(message)
            return
        }
        if let pairing { pairingDecisions.insert(pairing) }
        Task {
            do {
                let receipt = try await client.submit(command)
                if let outcome = earlyResults.removeValue(forKey: receipt.requestId) {
                    switch outcome {
                    case .applied: completion?(receipt); applied?()
                    case .failed(let message):
                        if let pairing { pairingDecisions.remove(pairing) }
                        failure?(); failed?(message)
                    }
                    return
                }
                pendingRequests.insert(receipt.requestId)
                if let pairing { requestPairings[receipt.requestId] = pairing }
                if let failure { failureHandlers[receipt.requestId] = failure }
                if let applied { appliedHandlers[receipt.requestId] = applied }
                if let failed { errorHandlers[receipt.requestId] = failed }
                completion?(receipt)
            } catch {
                self.error = error.userFacingMessage
                if let pairing { pairingDecisions.remove(pairing) }
                failure?(); failed?(error.userFacingMessage)
            }
        }
    }
    func reclaim() { engine?.emergencyReclaim() }
    /// 是否应该出现在 Dock 里：设置项，加上“远程应用窗口打开”这一覆盖。
    var dockIconVisible: Bool {
        if #available(macOS 14.2, *), RemoteApplicationDockController.shared.hasActiveApplication { return true }
        return !hideDockIcon
    }
    /// Applies the Dock visibility choice; call at launch and whenever it changes.
    func applyDockIconPolicy() {
        // WebView 模式下产品窗口属于 UI 子进程，Dock 图标归它：宿主再注册一个
        // regular 应用只会让同一个 bundle 出现两个图标。可见性经桥接推给子进程。
        if MyGoUIHost.shared.isActive {
            NSApp.setActivationPolicy(.accessory)
            MyGoUIHost.shared.pushDockPolicy(visible: dockIconVisible)
            return
        }
        NSApp.setActivationPolicy(dockIconVisible ? .regular : .accessory)
    }
    func setHideDockIcon(_ hidden: Bool) {
        hideDockIcon = hidden
        UserDefaults.standard.set(hidden, forKey: AppStore.hideDockIconDefaultsKey)
        applyDockIconPolicy()
    }
    func openSettings(tab: String? = nil) {
        // Preserve older settings links for features now in the main sidebar.
        if tab == "input" || tab == "microphone" {
            selectedPage = tab == "microphone" ? .microphone : .input
            openMain()
            return
        }
        settingsDestination = SettingsDestination(tab: tab, previous: settingsDestination)
        selectedPage = .settings
        if MyGoUIHost.shared.isActive {
            MyGoUIHost.shared.present(page: Page.settings.bridgeID,
                                      settingsCategory: settingsDestination.category.rawValue)
            return
        }
        openMain()
    }
    func openMain() {
        NSApp.activate(ignoringOtherApps: true)
        if MyGoUIHost.shared.isActive {
            MyGoUIHost.shared.present(page: selectedPage.bridgeID)
            return
        }
        if let window = NSApp.windows.first(where: { $0.identifier?.rawValue == "deskfuse-main" }) {
            if window.isMiniaturized { window.deminiaturize(nil) }
            window.makeKeyAndOrderFront(nil)
        }
        else { showMainWindow?() }
    }
    func showLocalDevice() {
        selectedPage = .devices
        deviceSelectionRequest &+= 1
    }

    /// 从健康中心进入“添加电脑”时，先回到设备页；设备页的添加入口保持唯一。
    func presentAddDevice() {
        showLocalDevice()
    }
    func confirmSelfTest() {
        guard selfTest else { return }
        Task {
            try? await Task.sleep(for: .seconds(1))
            do { try acknowledgeStartup(); NSApp.terminate(nil) }
            catch { self.error = error.userFacingMessage; NSApp.terminate(nil) }
        }
    }
    func terminate() -> NSApplication.TerminateReply {
        MicrophoneSharing.shared.stop()
        MouseButtonController.shared.stop()
        if #available(macOS 14.2, *) { RemoteApplicationManager.shared.stop() }
        caffeineController.disable()
        if quitting || client == nil { return .terminateNow }
        quitting = true; shelf?.stop(); transferWindow?.stop()
        wallpaperPublicationTask?.cancel()
        // 退出时不能走紧急收回：它会把本机作为被控方的启用意图持久化为
        // "用户已收回"，重启后主控的自动激活只会收到 PausedByUser，必须手动
        // 点“重连”才能恢复。输入由引擎关闭与输入服务退出时的 reclaim 释放。
        // AppKit already holds the main run loop inside applicationShouldTerminate,
        // and the native teardown can wait on a peer or on an install handoff. Input
        // control is released by the engine teardown, the update journal is durable,
        // and the helper only replaces the bundle once this process is gone — so exit
        // never waits on that teardown. A stalled shutdown must not keep the app (or
        // an update that is waiting for it) alive.
        if let client { Task { try? await client.shutdown() } }
        return .terminateNow
    }
}
enum StartupError: LocalizedError { case invalidTransaction; var errorDescription: String? { "缺少启动自检事务" } }
enum Page: String, CaseIterable, Identifiable {
    case devices = "设备", clipboard = "剪贴板", input = "键鼠共享", files = "局域网传输"
    case remoteApplications = "远程应用", microphone = "麦克风共享", mouse = "鼠标自定义"
    case settings = "设置"
    var id: Self { self }
    /// WebView 页面标识，与 `apps/deskfuse-go/frontend/src/App.vue` 的导航 id 一致。
    var bridgeID: String {
        switch self {
        case .devices: "devices"
        case .clipboard: "clipboard"
        case .input: "input"
        case .files: "files"
        case .remoteApplications: "remote_applications"
        case .microphone: "microphone"
        case .mouse: "mouse"
        case .settings: "settings"
        }
    }
    /// 侧栏功能分两组：日常协作的基础功能在上，进阶差异化功能（Beta）在下。
    static let primary: [Page] = [.clipboard, .input, .files]
    static let extended: [Page] = [.remoteApplications, .microphone, .mouse]
    var isBeta: Bool { Page.extended.contains(self) }
    /// 侧栏分组名，同时作为页面顶栏面包屑的第一级。
    var section: String {
        switch self {
        case .devices: "配置"
        case .settings: "设置"
        case .clipboard, .input, .files: "基础"
        case .remoteApplications, .microphone, .mouse: "高级"
        }
    }
    var symbol: String {
        switch self {
        case .devices: "laptopcomputer.and.iphone"
        case .clipboard: "doc.on.clipboard"
        case .input: "keyboard"
        case .files: "arrow.up.arrow.down.square"
        case .remoteApplications: "macwindow.on.rectangle"
        case .microphone: "mic"
        case .mouse: "computermouse"
        case .settings: "gearshape"
        }
    }
}
func byteCount(_ bytes: UInt64) -> String { ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .file) }

/// One-shot navigation state, not another copy of clipboard business data.
struct ClipboardNavigationRequest: Equatable {
    let token = UUID()
    let query: String
    let source: String?
    let selectedID: UInt64?
    var listID: String? = nil
}
