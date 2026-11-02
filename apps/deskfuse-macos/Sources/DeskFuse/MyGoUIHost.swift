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
        guard let path = value else {
            if let bundled = bundledExecutable.flatMap({ isExecutable($0.path) }) { return bundled }
            return developmentExecutable()
        }
        guard !path.isEmpty else { throw Failure.missingPath }
        guard path.hasPrefix("/") else { throw Failure.notAbsolute(path) }
        guard let url = isExecutable(path) else { throw Failure.notExecutable(path) }
        return url
    }

    /// 只有 DEBUG 构建会去仓库产物里找界面进程：`swift build` 出来的 app 不在
    /// 打包目录里，同级没有随包的 deskfuse-mygo。发布包永远只认随包二进制。
    private static func developmentExecutable() -> URL? {
        #if DEBUG
        guard let directory = Bundle.main.executableURL?.deletingLastPathComponent() else { return nil }
        return devExecutable(startingAt: directory)
        #else
        return nil
        #endif
    }

    /// 从给定目录逐层向上找仓库根（含 apps/deskfuse-go/go.mod），再返回可用的界面进程。
    static func devExecutable(startingAt directory: URL) -> URL? {
        var current: URL? = directory
        for _ in 0..<10 {
            guard let directory = current else { break }
            if FileManager.default.fileExists(atPath: directory.appendingPathComponent("apps/deskfuse-go/go.mod").path) {
                let candidates = [
                    directory.appendingPathComponent("target/mygo-webview/deskfuse-mygo"),
                    directory.appendingPathComponent("apps/deskfuse-go/deskfuse"),
                ]
                return candidates.lazy.compactMap { isExecutable($0.path) }.first
            }
            current = directory.deletingLastPathComponent()
        }
        return nil
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

    private func fail(expected: String? = nil) {
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

    /// 轻量模式：界面进程已被回收，核心（引擎、剪贴板同步、键鼠共享）仍在宿主里运行。
    private(set) var suspended = false
    private var idleMonitor: Task<Void, Never>?
    /// 最近一次界面上有窗口的时刻；界面收起后从此计时。
    private var lastScreenActivity = Date()
    /// 无窗口多久后回收界面进程。
    static let lightweightIdleSeconds: TimeInterval = 600
    static let idlePollSeconds: UInt64 = 30

    /// 界面进程空闲回收：窗口收起、应用不在前台持续 10 分钟后结束界面进程，
    /// 只留宿主持有的核心。任何界面请求都会重新拉起它。
    func startIdleMonitoring() {
        guard idleMonitor == nil else { return }
        lastScreenActivity = Date()
        idleMonitor = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(Self.idlePollSeconds))
                guard let self, !Task.isCancelled else { return }
                guard self.isActive, !self.stopping else { continue }
                if !AppStore.shared.lightweightMode {
                    // 关闭轻量模式后界面常驻：重置计时，重新开启时从头算。
                    self.lastScreenActivity = Date()
                    continue
                }
                if self.uiWindowsOnScreen() || NSApp.isActive {
                    self.lastScreenActivity = Date()
                    continue
                }
                if Date().timeIntervalSince(self.lastScreenActivity) >= Self.lightweightIdleSeconds {
                    self.suspendForLightweightMode()
                }
            }
        }
    }

    /// 界面进程里是否还有窗口留在屏幕上（含面板与浮窗）。
    private func uiWindowsOnScreen() -> Bool {
        guard let pid = childPID,
              let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] else {
            return false
        }
        return list.contains { info in
            guard (info[kCGWindowOwnerPID as String] as? Int) == Int(pid),
                  let bounds = info[kCGWindowBounds as String] as? [String: Any] else { return false }
            let width = (bounds["Width"] as? Double) ?? 0
            let height = (bounds["Height"] as? Double) ?? 0
            return width > 8 && height > 8
        }
    }

    private func suspendForLightweightMode() {
        guard isActive, !stopping else { return }
        NSLog("DeskFuse UI suspended after %d s without a window", Int(Self.lightweightIdleSeconds))
        suspended = true
        stop()
        // 界面进程退场后 Dock 图标回到宿主持有，用户点它就能重新拉起界面。
        AppStore.shared.applyDockIconPolicy()
    }

    /// 需要界面时重新拉起界面进程；`route` 是启动页，避免拉起后还要再发一次请求。
    private func resumeForUI(route: String?, panelFrame: String? = nil) {
        guard suspended || !isActive else { return }
        suspended = false
        lastScreenActivity = Date()
        startIfRequested(initialRoute: route, panelFrame: panelFrame)
    }

    /// Show the compact clipboard panel at a top-left origin in screen points.
    /// `y` is passed with the origin at the bottom (AppKit screen coordinates)
    /// so the WebView window can place itself without querying the display.
    func showPanel(x: Double, y: Double, width: Int, height: Int) {
        guard isActive else { return resumeWithPanel(x: x, y: y, width: width, height: height) }
        send(event: "panel.show", payload: ["x": Int(x), "y": Int(y), "width": width, "height": height])
    }

    /// Toggle keeps its state in the UI process, which also hides the panel when
    /// it loses focus.
    func togglePanel(x: Double, y: Double, width: Int, height: Int) {
        guard isActive else { return resumeWithPanel(x: x, y: y, width: width, height: height) }
        send(event: "panel.toggle", payload: ["x": Int(x), "y": Int(y), "width": width, "height": height])
    }

    func hidePanel() { send(event: "panel.hide") }

    /// 轻量模式下打开剪贴板面板：界面进程要重新启动，面板位置随启动参数带过去，
    /// 避免"先拉起主窗口再发 panel.show"闪一下。
    private func resumeWithPanel(x: Double, y: Double, width: Int, height: Int) {
        resumeForUI(route: "clipboard-panel",
                    panelFrame: "\(Int(x)),\(Int(y)),\(width),\(height)")
    }

    /// 界面进程可按需回收：任何界面请求先确保它在运行。
    private func ensureUI(route: String?) {
        guard !isActive else { return }
        resumeForUI(route: route)
    }

    /// 启动路由：`page` 或 `page:category`，由 Go 侧转成窗口 URL 的查询参数。
    static func route(page: String, category: String? = nil) -> String {
        guard let category, !category.isEmpty else { return page }
        return "\(page):\(category)"
    }

    private func send(event: String, payload: [String: Any] = [:]) {
        guard isActive, let writer else { return }
        do {
            let object: [String: Any] = ["v": 1, "event": event, "result": payload]
            writer.write(MyGoBridgeFrame.encode(try JSONSerialization.data(withJSONObject: object)))
        } catch { NSLog("MyGo window request encoding failed") }
    }

    func present(page: String, settingsCategory: String? = nil) {
        ensureUI(route: Self.route(page: page, category: settingsCategory))
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

    /// 界面进程是硬依赖：发布包里它总是与主程序并列存在，缺失或不可执行即为
    /// 安装损坏。这里不再退回到任何原生界面，直接报错退出，避免出现两套 UI。
    func startIfRequested(arguments: [String] = ProcessInfo.processInfo.arguments, initialRoute: String? = nil, panelFrame: String? = nil) {
        let executable: URL?
        do { executable = try MyGoUIOptions.executableURL(arguments: arguments) } catch {
            NSLog("DeskFuse MyGo UI not started: %@", String(describing: error))
            fail()
            return
        }
        guard let executable else {
            let expected = MyGoUIOptions.bundledExecutable?.path ?? "deskfuse-mygo"
            NSLog("DeskFuse MyGo UI missing: expected %@ (build the client with scripts/build-mygo-ui.sh)", expected)
            fail(expected: expected)
            return
        }
        start(executable: executable, initialRoute: initialRoute, panelFrame: panelFrame, engine: { await MainActor.run { AppStore.shared.client } }, snapshot: { try await MainActor.run { try MyGoSnapshot.encode(AppStore.shared) } }, invoke: { action, params in try await MyGoInvocation.run(action: action, params: params, store: AppStore.shared) })
        guard isActive else { return }
        AppStore.shared.applyDockIconPolicy()
        startAuxiliaryUpdates()
        startIdleMonitoring()
    }

    /// 没有界面进程就没有产品窗口：宿主自己也没有窗口可以显示错误，
    /// 因此用系统弹窗说明安装已损坏，然后退出。
    private func fail(expected: String? = nil) {
        NSApp.activate(ignoringOtherApps: true)
        let alert = NSAlert()
        alert.alertStyle = .critical
        alert.messageText = "DeskFuse 无法启动"
        var message = "界面组件缺失或不可执行，安装可能不完整。请从官网重新下载并安装 DeskFuse。"
        if let expected { message += "\n\n期望位置：\(expected)" }
        alert.informativeText = message
        alert.addButton(withTitle: "退出")
        alert.runModal()
        NSApp.terminate(nil)
    }

    func start(executable: URL, initialRoute: String? = nil, panelFrame: String? = nil, engine: @escaping MyGoEngineProvider, snapshot: MyGoSnapshotProvider? = nil, invoke: MyGoInvokeHandler? = nil) {
        guard process == nil else { return }
        // 轻量模式回收后再启动：清掉上一次 stop() 留下的退出标记。
        stopping = false
        let toChild = Pipe()
        let fromChild = Pipe()
        let process = Process()
        process.executableURL = executable
        process.arguments = []
        var environment = ProcessInfo.processInfo.environment.merging(
            [MyGoBridge.environmentKey: MyGoBridge.environmentValue]) { _, new in new }
        if let initialRoute { environment["DESKFUSE_INITIAL_ROUTE"] = initialRoute }
        if let panelFrame { environment["DESKFUSE_PANEL_FRAME"] = panelFrame }
        process.environment = environment
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
                // 界面进程就是产品窗口：它退出后宿主没有可显示的界面，
                // 不再重启也不再回落到原生窗口，直接结束，由系统记录退出原因。
                NSLog("DeskFuse MyGo UI exited with status %d", finished.terminationStatus)
                NSApp.terminate(nil)
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
