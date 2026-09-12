import SwiftUI
import AppKit
import Observation
import UniDeskEngine

@MainActor @Observable final class AppStore {
    static let shared = AppStore()
    var snapshot: EngineSnapshot?
    var error: String?
    var starting = true
    var selfTest = false
    var selectedPage: Page = .clipboard
    var clipboardNavigation: ClipboardNavigationRequest?
    var pendingRequests: Set<UInt64> = []
    var pairingDecisions: Set<String> = []
    var client: EngineClient?
    // These nonblocking native calls bypass the actor queue for emergency/drag callbacks.
    var engine: Engine?
    var shelf: ShelfController?
    var showMainWindow: (() -> Void)?
    var pairingWindow: PairingWindowController?
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
        do {
            let args = ProcessInfo.processInfo.arguments
            let transaction = args.firstIndex(of: "--update-self-test").flatMap { $0 + 1 < args.count ? args[$0 + 1] : nil }
            if args.contains("--update-self-test") && transaction == nil { throw StartupError.invalidTransaction }
            selfTest = try prepareStartup(transaction: transaction)
            if selfTest { starting = false; return }
        } catch { self.error = error.userFacingMessage; starting = false; return }
        Task {
            do {
                let engine = try await Task.detached { try Engine() }.value
                self.engine = engine
                await ImageWorkflowStore.shared.reload(engine: engine)
                UniDeskImageAppShortcuts.updateAppShortcutParameters()
                let client = EngineClient(engine: engine)
                self.client = client
                let (stream, continuation) = AsyncStream<EngineEvent>.makeStream()
                let relay = EventRelay(continuation)
                eventsTask = Task { [weak store = self] in
                    for await event in stream {
                        guard !Task.isCancelled else { break }
                        store?.consume(event)
                    }
                }
                try await client.listen(relay)
                shelf = ShelfController(store: self)
                pairingWindow = PairingWindowController(store: self)
                pairingWindow?.reconcile(snapshot?.pairing ?? [])
                starting = false
            } catch { self.error = error.userFacingMessage; starting = false }
        }
    }
    func consume(_ event: EngineEvent) {
        switch event {
        case .imageWorkflowRun(let run):
            ImageWorkflowStore.shared.consume(run)
        case .snapshot(let value):
            if value.revision >= (snapshot?.revision ?? 0) {
                snapshot = value
                pairingDecisions.formIntersection(Set(value.pairing.map(\.id)))
                shelf?.reconcileTransfers(value.transfers)
                pairingWindow?.reconcile(value.pairing)
            }
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
            Task { try? await Task.sleep(for: .milliseconds(150))
                let frontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier ?? 0
                do { _ = try replayPaste(targetPid: pid, currentPid: frontmost, changeCount: count) }
                catch { self.error = error.userFacingMessage }
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
            error = "引擎尚未就绪"; failure?(); failed?("引擎尚未就绪"); return
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
    func openMain() {
        NSApp.activate(ignoringOtherApps: true)
        if let window = NSApp.windows.first(where: { $0.identifier?.rawValue == "unidesk-main" }) { window.makeKeyAndOrderFront(nil) }
        else { showMainWindow?() }
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
        if quitting || client == nil { return .terminateNow }
        quitting = true; reclaim(); shelf?.stop()
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
    case clipboard = "剪贴板", input = "键鼠共享", devices = "设备", transfers = "传输"
    var id: Self { self }
    var symbol: String { switch self { case .clipboard: "clipboard"; case .input: "keyboard"; case .devices: "desktopcomputer"; case .transfers: "arrow.up.arrow.down" } }
}
func byteCount(_ bytes: UInt64) -> String { ByteCountFormatter.string(fromByteCount: Int64(clamping: bytes), countStyle: .file) }

/// One-shot navigation state, not another copy of clipboard business data.
struct ClipboardNavigationRequest: Equatable {
    let token = UUID()
    let query: String
    let kind: ClipboardKind
    let source: String?
    let selectedID: UInt64?
}
