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
    var selfTest = false
    /// A connected client may precede its first snapshot. Do not flash a
    /// failure page during that gap, or mask a real startup failure.
    var isEngineLoading: Bool {
        !selfTest && error == nil && (starting || (client != nil && snapshot == nil))
    }
    var selectedPage: Page = .clipboard {
        didSet { if selectedPage != .settings { workspacePage = selectedPage } }
    }
    /// Keep the workspace mounted while settings occupies the detail area.
    private(set) var workspacePage: Page = .clipboard
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
    /// Launch-time guidance for permissions used by local features.
    let permissionPrompt = PermissionPromptController()
    let onboarding = OnboardingController()
    let previousInstallation = PreviousInstallationController()
    var showMainWindow: (() -> Void)?
    var pairingWindow: PairingWindowController?
    private var transferWindow: TransferProgressWindowController?
    private var eventsTask: Task<Void, Never>?
    private enum CommandOutcome { case applied, failed(String) }
    private var earlyResults: [UInt64: CommandOutcome] = [:]
    private var appliedHandlers: [UInt64: () -> Void] = [:]
    private var errorHandlers: [UInt64: (String) -> Void] = [:]
    private var failureHandlers: [UInt64: () -> Void] = [:]
    private var requestPairings: [UInt64: String] = [:]
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
                let engine = try await Task.detached { try Engine() }.value
                recordStartup("engine_ready")
                self.engine = engine
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

    /// Capture local wallpaper previews for the layout UI and publish them to
    /// peers. The page refreshes its temporary images when it is opened; the
    /// launch-time call keeps remote cards populated before that page is shown.
    func refreshDisplayWallpapers() async {
        guard !Task.isCancelled, let result = await displayWallpaperPublisher.refresh() else { return }
        guard !Task.isCancelled else { return }
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
                snapshot = value
                if shouldPublishWallpaper { publishDisplayWallpapers() }
                if #available(macOS 14.2, *), !value.input.canOperateRemoteApplications {
                    RemoteApplicationManager.shared.reconcileViewerRole()
                }
                pairingDecisions.formIntersection(Set(value.pairing.map(\.id)))
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
    /// Applies the Dock visibility choice; call at launch and whenever it changes.
    func applyDockIconPolicy() {
        if #available(macOS 14.2, *), RemoteApplicationDockController.shared.hasActiveApplication {
            NSApp.setActivationPolicy(.regular)
            return
        }
        NSApp.setActivationPolicy(hideDockIcon ? .accessory : .regular)
    }
    func setHideDockIcon(_ hidden: Bool) {
        hideDockIcon = hidden
        UserDefaults.standard.set(hidden, forKey: AppStore.hideDockIconDefaultsKey)
        applyDockIconPolicy()
    }
    func openSettings(tab: String? = nil, clipboardSegment: String? = nil) {
        // The former input settings only duplicated the main keyboard/mouse page.
        if tab == "input" {
            selectedPage = .input
            openMain()
            return
        }
        settingsDestination = SettingsDestination(tab: tab, clipboardSegment: clipboardSegment,
                                                  previous: settingsDestination)
        selectedPage = .settings
        openMain()
    }
    func openMain() {
        NSApp.activate(ignoringOtherApps: true)
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
        quitting = true; reclaim(); shelf?.stop(); transferWindow?.stop()
        wallpaperPublicationTask?.cancel()
        // AppKit already holds the main run loop inside applicationShouldTerminate,
        // and the native teardown can wait on a peer or on an install handoff. Input
        // control is released above, the update journal is durable, and the helper
        // only replaces the bundle once this process is gone — so exit never waits
        // on that teardown. A stalled shutdown must not keep the app (or an update
        // that is waiting for it) alive.
        if let client { Task { try? await client.shutdown() } }
        return .terminateNow
    }
}
enum StartupError: LocalizedError { case invalidTransaction; var errorDescription: String? { "缺少启动自检事务" } }
enum Page: String, CaseIterable, Identifiable {
    case clipboard = "剪贴板", input = "键鼠共享", mouse = "鼠标", remoteApplications = "远程应用", devices = "设备", transfers = "传输", lanTransfer = "分享与传输", settings = "设置"
    /// The transfer queue is presented together with LAN sharing. Keep the
    /// legacy case for deep links and existing command code, but do not expose
    /// a second top-level destination in the redesigned sidebar.
    static var allCases: [Page] { [.clipboard, .input, .mouse, .remoteApplications, .devices, .lanTransfer, .settings] }
    var id: Self { self }
    var symbol: String { switch self { case .clipboard: "doc.on.clipboard"; case .input: "rectangle.on.rectangle"; case .mouse: "computermouse"; case .remoteApplications: "square.grid.2x2"; case .devices: "laptopcomputer.and.iphone"; case .transfers: "arrow.left.arrow.right"; case .lanTransfer: "paperplane"; case .settings: "gearshape" } }
    /// 侧栏彩色方块图标的识别色，与设置页同名功能保持同色。
    var color: Color { switch self { case .clipboard: .blue; case .input: .purple; case .mouse: .orange; case .remoteApplications: .teal; case .devices: .green; case .transfers: .orange; case .lanTransfer: .orange; case .settings: .gray } }
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
