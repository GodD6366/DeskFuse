import AppKit
import Foundation
import Darwin

/// `--mygo-ui <executable>` overrides the UI client. Without it the app uses the
/// client bundled in Contents/MacOS, and falls back to the native window only in
/// development bundles that do not contain one.
enum MyGoUIOptions {
    static let flag = "--mygo-ui"
    static let environmentKey = "DESKFUSE_MYGO_UI"

    enum Failure: Error, Equatable {
        case missingPath
        case notAbsolute(String)
        case notExecutable(String)
    }

    /// The bundled client ships beside the app executable.
    static var bundledExecutable: URL? {
        Bundle.main.executableURL?
            .deletingLastPathComponent()
            .appendingPathComponent("deskfuse-mygo")
    }

    /// nil when no client is requested and none is bundled; throws when an
    /// explicit path is present but unusable.
    static func executableURL(arguments: [String]) throws -> URL? {
        var value: String?
        if let index = arguments.firstIndex(of: flag) {
            guard index + 1 < arguments.count, !arguments[index + 1].hasPrefix("--") else { throw Failure.missingPath }
            value = arguments[index + 1]
        } else if let inline = arguments.first(where: { $0.hasPrefix(flag + "=") }) {
            value = String(inline.dropFirst(flag.count + 1))
        } else if let override = ProcessInfo.processInfo.environment[environmentKey], !override.isEmpty {
            value = override
        }
        guard let path = value else { return bundledExecutable.flatMap { isExecutable($0.path) } }
        guard !path.isEmpty else { throw Failure.missingPath }
        guard path.hasPrefix("/") else { throw Failure.notAbsolute(path) }
        guard let url = isExecutable(path) else { throw Failure.notExecutable(path) }
        return url
    }

    private static func isExecutable(_ path: String) -> URL? {
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), !isDirectory.boolValue,
              FileManager.default.isExecutableFile(atPath: path) else { return nil }
        return URL(fileURLWithPath: path)
    }
}

/// Serializes frame writes to the child's stdin so the actor never blocks on a full pipe.
final class MyGoPipeWriter: @unchecked Sendable {
    private var descriptor: Int32
    private let queue = DispatchQueue(label: "com.deskfuse.mygo-bridge.writer")
    private let lock = NSLock()
    private var pending = 0
    private var failed = false
    private let limit: Int
    private let onFailure: @Sendable () -> Void

    /// Takes ownership of `descriptor`, which must have F_SETNOSIGPIPE set so a
    /// vanished child yields EPIPE, not SIGPIPE.
    init(descriptor: Int32, limit: Int = 4 * MyGoBridge.maxReplyBytes, onFailure: @escaping @Sendable () -> Void) {
        self.descriptor = descriptor
        self.limit = limit
        self.onFailure = onFailure
    }

    func write(_ data: Data) {
        let accepted = lock.withLock { () -> Bool in
            guard !failed, pending + data.count <= limit else { return false }
            pending += data.count
            return true
        }
        guard accepted else { fail(); return }
        queue.async { [self] in
            let ok = descriptor >= 0 && data.withUnsafeBytes { raw -> Bool in
                var offset = 0
                while offset < raw.count {
                    let written = Darwin.write(descriptor, raw.baseAddress! + offset, raw.count - offset)
                    if written < 0 { if errno == EINTR { continue }; return false }
                    offset += written
                }
                return true
            }
            lock.withLock { pending -= data.count }
            if !ok { fail() }
        }
    }

    /// Writes already queued still go out; later ones are refused. Then the descriptor closes.
    func close() {
        lock.withLock { failed = true }
        queue.async { [self] in
            if descriptor >= 0 { Darwin.close(descriptor); descriptor = -1 }
        }
    }

    private func fail() {
        let first = lock.withLock { () -> Bool in
            defer { failed = true }
            return !failed
        }
        if first { onFailure() }
    }
}

/// Owns the optional MyGo UI child process and its stdio bridge. It never creates
/// an engine: queries go to the engine the app already started.
@MainActor final class MyGoUIHost {
    static let shared = MyGoUIHost()

    private var process: Process?
    private(set) var stopping = false
    private var server: MyGoBridgeServer?
    private var writer: MyGoPipeWriter?
    private(set) var childPID: Int32?
    private var auxiliaryTask: Task<Void, Never>?
    private var lastAuxiliary = ""
    var isActive: Bool { process?.isRunning == true }

    /// Show the compact clipboard panel at a top-left origin in screen points.
    /// `y` is passed with the origin at the bottom (AppKit screen coordinates)
    /// so the WebView window can place itself without querying the display.
    func showPanel(x: Double, y: Double, width: Int, height: Int) {
        send(event: "panel.show", payload: ["x": Int(x), "y": Int(y), "width": width, "height": height])
    }

    /// Toggle keeps its state in the UI process, which also hides the panel when
    /// it loses focus.
    func togglePanel(x: Double, y: Double, width: Int, height: Int) {
        send(event: "panel.toggle", payload: ["x": Int(x), "y": Int(y), "width": width, "height": height])
    }

    func hidePanel() { send(event: "panel.hide") }

    private func send(event: String, payload: [String: Any] = [:]) {
        guard isActive, let writer else { return }
        do {
            let object: [String: Any] = ["v": 1, "event": event, "result": payload]
            writer.write(MyGoBridgeFrame.encode(try JSONSerialization.data(withJSONObject: object)))
        } catch { NSLog("MyGo window request encoding failed") }
    }

    func present(page: String, settingsCategory: String? = nil) {
        guard isActive, let writer else { return }
        var result: [String: Any] = ["page": page]
        if let settingsCategory { result["settings_category"] = settingsCategory }
        NSApp.activate(ignoringOtherApps: true)
        do {
            let payload = try JSONSerialization.data(withJSONObject: ["v": 1, "event": "ui.present", "result": result])
            writer.write(MyGoBridgeFrame.encode(payload))
        } catch { NSLog("MyGo window request encoding failed") }
    }

    /// Dock 图标归持有产品窗口的 UI 进程：宿主只把计算结果推给它，自己保持
    /// accessory，否则同一个 bundle 会出现两个 Dock 图标。
    func pushDockPolicy(visible: Bool) {
        send(event: "ui.dock", payload: ["visible": visible])
    }

    func startIfRequested(arguments: [String] = ProcessInfo.processInfo.arguments) {
        let executable: URL?
        do { executable = try MyGoUIOptions.executableURL(arguments: arguments) } catch {
            // Diagnostics go to stderr via NSLog; stdout stays free of anything but frames.
            NSLog("DeskFuse MyGo UI not started: %@", String(describing: error))
            return
        }
        guard let executable else { return }
        start(executable: executable, engine: { await MainActor.run { AppStore.shared.client } }, snapshot: { try await MainActor.run { try MyGoSnapshot.encode(AppStore.shared) } }, invoke: { action, params in try await MyGoInvocation.run(action: action, params: params, store: AppStore.shared) })
        // The WebView owns the product window; the native scene must not also open one.
        if isActive {
            MyGoUIHost.suppressNativeMainWindow()
            AppStore.shared.concealNativeAuxiliaryWindows()
            AppStore.shared.applyDockIconPolicy()
            startAuxiliaryUpdates()
        }
    }

    func start(executable: URL, engine: @escaping MyGoEngineProvider, snapshot: MyGoSnapshotProvider? = nil, invoke: MyGoInvokeHandler? = nil) {
        guard process == nil else { return }
        let toChild = Pipe()
        let fromChild = Pipe()
        let process = Process()
        process.executableURL = executable
        process.arguments = []
        process.environment = ProcessInfo.processInfo.environment.merging(
            [MyGoBridge.environmentKey: MyGoBridge.environmentValue]) { _, new in new }
        process.standardInput = toChild
        process.standardOutput = fromChild
        process.standardError = FileHandle.standardError
        // The writer and the reader thread own private descriptors, so closing or
        // releasing the Pipe handles can never pull a descriptor out from under them.
        let writeDescriptor = dup(toChild.fileHandleForWriting.fileDescriptor)
        let readDescriptor = dup(fromChild.fileHandleForReading.fileDescriptor)
        guard writeDescriptor >= 0, readDescriptor >= 0 else {
            NSLog("DeskFuse MyGo UI not started: dup failed (%d)", errno)
            if writeDescriptor >= 0 { Darwin.close(writeDescriptor) }
            if readDescriptor >= 0 { Darwin.close(readDescriptor) }
            return
        }
        _ = fcntl(writeDescriptor, F_SETNOSIGPIPE, 1)

        let (chunks, continuation) = AsyncStream<Data>.makeStream()
        let writeEnd = toChild.fileHandleForWriting
        let writer = MyGoPipeWriter(descriptor: writeDescriptor) { continuation.finish() }
        let server = MyGoBridgeServer(engine: engine, send: { writer.write($0) }, onClose: {
            // Closing stdin tells the child the app is done; it must not outlive the bridge.
            writer.close()
            try? writeEnd.close()
            continuation.finish()
        }, snapshot: snapshot, invoke: invoke)
        Thread.detachNewThread {
            defer { Darwin.close(readDescriptor) }
            var buffer = [UInt8](repeating: 0, count: 64 * 1024)
            while true {
                let count = read(readDescriptor, &buffer, buffer.count)
                if count < 0, errno == EINTR { continue }
                guard count > 0 else { break }
                continuation.yield(Data(buffer[0..<count]))
            }
            continuation.finish()
        }
        Task {
            for await chunk in chunks { await server.receive(chunk) }
            await server.close()
        }
        process.terminationHandler = { finished in
            NSLog("DeskFuse MyGo UI exited with status %d", finished.terminationStatus)
            continuation.finish()
            Task { @MainActor in
                let host = MyGoUIHost.shared
                // 退出流程会先 stop() 并清空进程；这里只处理子进程自己结束。
                guard host.process === process, !host.stopping else { return }
                host.process = nil
                host.writer = nil
                host.server = nil
                host.childPID = nil
                host.auxiliaryTask?.cancel()
                host.auxiliaryTask = nil
                if finished.terminationReason == .exit, finished.terminationStatus == 0 {
                    // 用户在 UI 里正常退出：宿主不再空跑。
                    NSApp.terminate(nil)
                    return
                }
                // 界面异常退出：恢复宿主自己的 Dock 身份并回落到原生窗口，
                // 用户仍能打开设置或退出，而不是面对一个没有窗口的应用；
                // 录制快捷键时被暂停的全局热键也要恢复。
                ClipboardPanelController.shared.suspendShortcut(false)
                AppStore.shared.applyDockIconPolicy()
                NSApp.windows.first { $0.identifier?.rawValue == "deskfuse-main" }?.makeKeyAndOrderFront(nil)
                NSApp.activate(ignoringOtherApps: true)
            }
        }
        do { try process.run() } catch {
            NSLog("DeskFuse MyGo UI failed to launch: %@", error.localizedDescription)
            continuation.finish()
            writer.close()
            return
        }
        // The child owns its copies now; keep only the ends the app uses.
        try? toChild.fileHandleForReading.close()
        try? fromChild.fileHandleForWriting.close()
        self.writer = writer
        self.childPID = process.processIdentifier
        self.process = process
        self.server = server
    }

    /// Push the state of the floating windows the native app used to own, so the
    /// UI process can show, position and hide them without asking for snapshots.
    /// Only changes are sent; the window itself renders from the same snapshot.
    private func startAuxiliaryUpdates() {
        auxiliaryTask?.cancel()
        lastAuxiliary = ""
        auxiliaryTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let self, self.isActive else { return }
                self.pushAuxiliaryState()
                try? await Task.sleep(for: .milliseconds(500))
            }
        }
    }

    private func pushAuxiliaryState() {
        let snapshot = AppStore.shared.snapshot
        let pairing = snapshot?.pairing.filter(\.incoming).map(\.id) ?? []
        let transfers = snapshot?.transfers.map { ["id": $0.id, "phase": String(describing: $0.phase)] } ?? []
        guard let data = try? JSONSerialization.data(withJSONObject: ["pairing": pairing, "transfers": transfers]),
              let text = String(data: data, encoding: .utf8), text != lastAuxiliary,
              let payload = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return }
        lastAuxiliary = text
        send(event: "ui.aux", payload: payload)
    }

    /// Close the SwiftUI main window once, after the scene has created it. The
    /// WebView is the product window whenever the client is running.
    static func suppressNativeMainWindow() {
        let close = {
            NSApp.windows.first { $0.identifier?.rawValue == "deskfuse-main" }?.orderOut(nil)
        }
        close()
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(400))
            close()
        }
    }

    /// 旧安装迁移提示仍挂在原生主窗口的 sheet 上；WebView 模式下主窗口被收起，
    /// 若直接呈现 sheet 用户看不到也无法完成导入，引擎会一直停在等待状态。
    /// 因此需要原生 sheet 时临时把主窗口放回前台，结束后再收起。
    static func revealNativeMainWindow() {
        guard MyGoUIHost.shared.isActive,
              let window = NSApp.windows.first(where: { $0.identifier?.rawValue == "deskfuse-main" }) else { return }
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    static func suppressNativeMainWindowIfActive() {
        if MyGoUIHost.shared.isActive { suppressNativeMainWindow() }
    }

    /// App is quitting: stop serving, close the pipes and terminate the child.
    func stop() {
        guard let process else { return }
        stopping = true
        auxiliaryTask?.cancel()
        auxiliaryTask = nil
        self.writer = nil
        self.childPID = nil
        self.process = nil
        self.server = nil

        if process.isRunning { process.terminate() }
    }
}
