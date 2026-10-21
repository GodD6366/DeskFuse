import Foundation
import Observation
import DeskFuseEngine
import DeskFuseRemoteApplication

/// A prepared launch received from the Windows service. The ticket authorizes
/// exactly one QUIC tunnel; the `.rdp` document's logical host is never dialed.
struct RemoteRdpReady: Sendable {
    let name: String
    let ticket: Data
    let rdp: String
    let host: RemoteRdpHost
}

enum RemoteRdpCatalogError: LocalizedError {
    case unavailable(String)
    case prepareBusy
    case disconnected

    var errorDescription: String? {
        switch self {
        case .unavailable(let message): message
        case .prepareBusy: "上一个应用仍在启动，请稍后重试。"
        case .disconnected: remoteRdpDisconnectedMessage
        }
    }
}

let remoteRdpDisconnectedMessage = "Windows 应用连接已断开，请点击“刷新”重试。"

/// Catalog-only connection to one Windows host. Pixels and input live on the
/// host-scoped RDP tunnel owned by `RemoteRdpHostConnection`; closing this
/// browser connection never disconnects an open RDP session.
@available(macOS 14.2, *)
@MainActor @Observable final class RemoteRdpSession {
    enum Phase: Equatable { case idle, connecting, ready, failed(String) }
    private(set) var phase: Phase = .idle
    private(set) var apps: [RemoteRdpApp] = []
    private(set) var unavailableReason: String?
    private(set) var host: RemoteRdpHost?
    let engine: Engine
    let peerID: String
    let peerName: String
    var savedCredential: RemoteRDPCredentialStore.Credential? {
        RemoteRDPCredentialStore.load(peerID: peerID)
    }

    @ObservationIgnored private var channel: RemoteApplicationChannel?
    @ObservationIgnored private var reader: Task<Void, Never>?
    @ObservationIgnored private var generation = UUID()
    @ObservationIgnored private var connectTask: Task<Void, Never>?
    @ObservationIgnored private var pending: PendingPrepare?
    @ObservationIgnored private let writer = DispatchQueue(label: "DeskFuse.rdp.catalog.send")

    private struct PendingPrepare {
        let name: String
        let continuation: CheckedContinuation<RemoteRdpReady, Error>
    }

    init(engine: Engine, peerID: String, peerName: String) {
        self.engine = engine
        self.peerID = peerID
        self.peerName = peerName
    }

    /// Coalesces picker and host-connection callers onto one QUIC stream.
    func connect() async {
        guard RemoteApplicationManager.shared.enabled else { return }
        if let connectTask {
            await connectTask.value
            return
        }
        let task = Task { await self.performConnect() }
        connectTask = task
        await task.value
        connectTask = nil
    }

    private func performConnect() async {
        stop()
        let token = generation
        phase = .connecting
        let engine = engine, peerID = peerID
        do {
            let channel = try await Task.detached { try engine.openRemoteApplication(peerId: peerID) }.value
            guard generation == token, !Task.isCancelled else { channel.close(); return }
            self.channel = channel
            // Windows sends the initial catalog without a request. Blocking
            // receives must never run on the main actor's cooperative executor.
            reader = Task.detached { [weak self] in
                while !Task.isCancelled {
                    do {
                        guard let data = try channel.receive(waitMs: 250) else { continue }
                        let message = try JSONDecoder().decode(RemoteRdpMessage.self, from: data)
                        await self?.handle(message, token: token)
                    } catch {
                        await self?.fail(Self.displayMessage(for: error), token: token)
                        return
                    }
                }
            }
        } catch {
            guard generation == token else { return }
            fail(Self.displayMessage(for: error), token: token)
        }
    }

    func refresh() {
        if channel != nil { send(.catalog(version: RemoteRdpRequest.version)) }
        else { Task { await connect() } }
    }

    /// Registers `name` on the Windows host and returns a single-use tunnel
    /// ticket. Repeated prepares reuse the same catalog stream.
    func prepare(_ name: String) async throws -> RemoteRdpReady {
        if phase != .ready { await connect() }
        // `connect` returns once the stream is open; the initial catalog (which
        // proves the service accepted the version) arrives asynchronously.
        let deadline = ContinuousClock.now + .seconds(10)
        while phase == .idle || phase == .connecting {
            guard ContinuousClock.now < deadline else {
                throw RemoteRdpCatalogError.unavailable("读取 Windows 应用目录超时，请刷新后重试。")
            }
            try? await Task.sleep(for: .milliseconds(50))
        }
        guard channel != nil, case .ready = phase else {
            throw RemoteRdpCatalogError.unavailable(unavailableReason ?? "Windows 应用目录尚未就绪。")
        }
        guard pending == nil else { throw RemoteRdpCatalogError.prepareBusy }
        let token = generation
        return try await withCheckedThrowingContinuation { continuation in
            pending = PendingPrepare(name: name, continuation: continuation)
            send(.prepare(version: RemoteRdpRequest.version, name: name))
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(15))
                self?.prepareTimedOut(name: name, token: token)
            }
        }
    }

    private func send(_ request: RemoteRdpRequest) {
        guard let channel else { return }
        let token = generation
        writer.async { [weak self] in
            do { try channel.send(bytes: JSONEncoder().encode(request)) }
            catch { Task { @MainActor [weak self] in self?.fail(Self.displayMessage(for: error), token: token) } }
        }
    }

    func stop() {
        generation = UUID()
        reader?.cancel(); reader = nil
        channel?.close(); channel = nil
        resumePending(throwing: RemoteRdpCatalogError.disconnected)
        apps = []; unavailableReason = nil; host = nil
        phase = .idle
    }

    private func prepareTimedOut(name: String, token: UUID) {
        guard token == generation, pending?.name == name else { return }
        resumePending(throwing: RemoteRdpCatalogError.unavailable("Windows 应用启动请求超时，请刷新后重试。"))
    }

    private func resumePending(throwing error: Error) {
        guard let pending else { return }
        self.pending = nil
        pending.continuation.resume(throwing: error)
    }

    private func fail(_ message: String, token: UUID) {
        guard generation == token else { return }
        stop()
        phase = .failed(message)
    }

    private nonisolated static func displayMessage(for error: Error) -> String {
        // 解码错误的 localizedDescription 只有系统通用英文文案，必须按类型映射。
        if case RemoteRdpProtocolError.unsupportedVersion(let version) = error {
            return version < RemoteRdpMessage.version
                ? "Windows 端的 DeskFuse 版本过旧，请先在 Windows 上升级 DeskFuse 服务。"
                : "Mac 端的 DeskFuse 版本过旧，请先升级本机 DeskFuse。"
        }
        if error is DecodingError {
            return "Windows 端返回的应用目录无法识别，请确认两端 DeskFuse 版本一致。"
        }
        let message = error.userFacingMessage
        if message.localizedCaseInsensitiveContains("stream reset by peer")
            || message.contains("视频通道读取失败")
            || message.contains("视频通道消息不完整")
            || message.contains("远程应用通道读取失败")
            || message.contains("远程应用通道消息不完整") {
            return remoteRdpDisconnectedMessage
        }
        return message
    }

    private func handle(_ message: RemoteRdpMessage, token: UUID) {
        guard token == generation else { return }
        switch message {
        case .catalog(_, let available, let reason, let host, let apps):
            self.host = host
            unavailableReason = available ? nil : (reason ?? Self.hostFailureMessage(host))
            self.apps = available ? apps : []
            phase = .ready
        case .ready(_, let name, let ticket, let rdp, let host):
            guard let pending, pending.name == name else { return }
            self.pending = nil
            self.host = host
            pending.continuation.resume(returning: RemoteRdpReady(name: name, ticket: ticket, rdp: rdp, host: host))
        case .error(_, _, let message):
            if pending != nil {
                resumePending(throwing: RemoteRdpCatalogError.unavailable(message))
            } else {
                fail(message, token: token)
            }
        }
    }

    private static func hostFailureMessage(_ host: RemoteRdpHost) -> String {
        let failures = host.failedChecks.map(\.message).filter { !$0.isEmpty }
        return failures.isEmpty ? "Windows 远程桌面预检未通过" : failures.joined(separator: "\n")
    }
}
