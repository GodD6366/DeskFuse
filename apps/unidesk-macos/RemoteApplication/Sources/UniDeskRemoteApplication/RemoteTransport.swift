import Foundation
@preconcurrency import Network
@preconcurrency import Security
import CryptoKit

extension RemoteWire {
    public static func key() -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else { return "" }
        return Data(bytes).base64EncodedString()
    }
    public static func parameters(code: String, identity: RemoteTLSIdentity? = nil) throws -> NWParameters {
        guard let key = Data(base64Encoded: code.trimmingCharacters(in: .whitespacesAndNewlines)), key.count == 64 else {
            throw RemoteApplicationError.unsupported("请完整粘贴 B 端生成的连接密钥")
        }
        let options = NWProtocolQUIC.Options(alpn: ["unidesk-spike-v1"])
        options.direction = .bidirectional
        options.idleTimeout = 5_000
        options.initialMaxData = 32 * 1024 * 1024
        options.initialMaxStreamDataBidirectionalLocal = 32 * 1024 * 1024
        options.initialMaxStreamDataBidirectionalRemote = 32 * 1024 * 1024
        if let identity {
            sec_protocol_options_set_local_identity(options.securityProtocolOptions, identity.identity)
        } else {
            let expected = Data(key.prefix(32))
            sec_protocol_options_set_verify_block(options.securityProtocolOptions, { _, trust, complete in
                let native = sec_trust_copy_ref(trust).takeRetainedValue()
                guard let certificates = SecTrustCopyCertificateChain(native) as? [SecCertificate], let certificate = certificates.first else {
                    complete(false); return
                }
                // Trust exactly the certificate carried by B's out-of-band code.
                // A changed host certificate fails even on the same IP address.
                let actual = Data(SHA256.hash(data: SecCertificateCopyData(certificate) as Data))
                complete(actual == expected)
            }, DispatchQueue(label: "uniDesk.spike.tls"))
        }
        sec_protocol_options_set_tls_resumption_enabled(options.securityProtocolOptions, false)
        sec_protocol_options_set_tls_tickets_enabled(options.securityProtocolOptions, false)
        let parameters = NWParameters(quic: options)
        parameters.includePeerToPeer = true
        return parameters
    }
}

/// Standalone P0 QUIC connection, deliberately separate from production ports.
/// One bounded compressed stream; input travels in the reverse direction and
/// never waits behind outgoing video. Production multiplexing remains separate.
public final class RemotePeer: @unchecked Sendable {
    private let connection: NWConnection
    private let queue = DispatchQueue(label: "uniDesk.spike.quic", qos: .userInteractive)
    private let lock = NSLock()
    private var pending: [RemoteMessage] = []
    private var mediaBytes = 0
    private var scheduled = false
    private var stopped = false
    private var waitingForKey = false
    private let receive: @Sendable (RemoteMessage) -> Void
    private let state: @Sendable (String, Bool) -> Void
    private let serverToken: String?
    private var authenticated: Bool
    private let keyNeeded: @Sendable () -> Void
    public init(connection: NWConnection, serverToken: String? = nil, receive: @escaping @Sendable (RemoteMessage) -> Void,
                state: @escaping @Sendable (String, Bool) -> Void, keyNeeded: @escaping @Sendable () -> Void = {}) {
        self.serverToken = serverToken; authenticated = serverToken == nil
        self.connection = connection; self.receive = receive; self.state = state; self.keyNeeded = keyNeeded
    }
    public convenience init(host: String, port: UInt16, code: String, receive: @escaping @Sendable (RemoteMessage) -> Void,
                            state: @escaping @Sendable (String, Bool) -> Void) throws {
        let connection = NWConnection(host: NWEndpoint.Host(host), port: NWEndpoint.Port(rawValue: port)!,
                                      using: try RemoteWire.parameters(code: code))
        self.init(connection: connection, receive: receive, state: state)
    }
    public func start() {
        connection.stateUpdateHandler = { [weak self] value in
            guard let self else { return }
            switch value {
            case .ready:
                if self.authenticated { self.state("QUIC 已连接", true) }
                self.readHeader()
            case .failed(let error): self.fail("连接失败：\(error.localizedDescription)")
            case .waiting(let error): self.fail("网络不可用：\(error.localizedDescription)")
            case .cancelled: break
            default: break
            }
        }
        connection.start(queue: queue)
        queue.asyncAfter(deadline: .now() + 10) { [weak self] in
            guard let self, (self.connection.state != .ready || !self.authenticated) else { return }
            self.fail("连接超时，请检查地址、密钥及 B 端防火墙")
        }
    }
    public func send(_ message: RemoteMessage) {
        var needKey = false, overflow = false
        let wake = lock.withLock { () -> Bool in
            guard !stopped else { return false }
            if message.kind == .surface {
                pending.removeAll { $0.kind == .frame }; mediaBytes = 0; waitingForKey = true
            }
            if let frame = message.frame {
                if waitingForKey && !frame.keyFrame { return false }
                if pending.filter({ $0.kind == .frame }).count >= 3 || mediaBytes + frame.bytes.count > 24 * 1024 * 1024 {
                    pending.removeAll { $0.kind == .frame }; mediaBytes = 0; waitingForKey = true; needKey = true
                    if !frame.keyFrame { return false }
                }
                if frame.keyFrame { waitingForKey = false }
                mediaBytes += frame.bytes.count
            } else if message.kind == .input, message.input?.kind == .move,
                      pending.last?.kind == .input, pending.last?.input?.kind == .move,
                      pending.last?.identity == message.identity {
                pending[pending.count - 1] = message; return false
            }
            if pending.count >= 128 { overflow = true; return false }
            pending.append(message)
            guard !scheduled else { return false }
            scheduled = true; return true
        }
        if needKey { keyNeeded() }
        if overflow { fail("控制队列超限，连接已关闭"); return }
        if wake { queue.async { [weak self] in self?.drain() } }
    }
    private func drain() {
        let message = lock.withLock { () -> RemoteMessage? in
            guard !stopped, !pending.isEmpty else { scheduled = false; return nil }
            let value = pending.removeFirst(); mediaBytes -= value.frame?.bytes.count ?? 0; return value
        }
        guard let message else { return }
        do {
            let data = try RemoteWire.encode(message)
            connection.send(content: data, completion: .contentProcessed { [weak self] error in
                guard let self else { return }
                if let error { self.fail(error.localizedDescription) } else { self.drain() }
            })
        } catch { fail(error.localizedDescription) }
    }
    private func readHeader() {
        read(count: 4) { [weak self] header in
            guard let self else { return }
            let length = header.reduce(0) { ($0 << 8) | Int($1) }
            guard length > 0, length <= RemoteWire.maximum else { self.fail("非法消息长度"); return }
            self.read(count: length) { [weak self] data in
                guard let self else { return }
                do {
                    let message = try RemoteWire.decode(data)
                    if !self.authenticated {
                        guard message.kind == .hello, let token = message.text, let expected = self.serverToken,
                              RemoteTLSIdentity.matches(token, expected) else { self.fail("连接密钥不匹配"); return }
                        self.authenticated = true; self.state("已验证 A 端连接", true)
                    } else { self.receive(message) }
                    self.readHeader()
                }
                catch { self.fail(error.localizedDescription) }
            }
        }
    }
    private func read(count: Int, accumulated: Data = Data(), complete: @escaping @Sendable (Data) -> Void) {
        let remaining = count - accumulated.count
        connection.receive(minimumIncompleteLength: 1, maximumLength: remaining) { [weak self] data, _, ended, error in
            guard let self, !self.lock.withLock({ self.stopped }) else { return }
            if let error { self.fail(error.localizedDescription); return }
            var result = accumulated; if let data { result.append(data) }
            if result.count == count { complete(result) }
            else if ended { self.fail("对端已断开") }
            else { self.read(count: count, accumulated: result, complete: complete) }
        }
    }
    private func fail(_ message: String) {
        let first = lock.withLock { () -> Bool in
            guard !stopped else { return false }; stopped = true; pending.removeAll(); mediaBytes = 0; return true
        }
        guard first else { return }
        connection.cancel(); state(message, false)
    }
    public func stop() {
        lock.withLock { stopped = true; pending.removeAll(); mediaBytes = 0 }
        connection.cancel()
    }
}

public final class RemoteListener: @unchecked Sendable {
    private let listener: NWListener
    private let identity: RemoteTLSIdentity
    private let queue = DispatchQueue(label: "uniDesk.spike.listener")
    public init(port: UInt16, identity: RemoteTLSIdentity, accepted: @escaping @Sendable (NWConnection) -> Void,
                status: @escaping @Sendable (String) -> Void) throws {
        self.identity = identity
        listener = try NWListener(using: RemoteWire.parameters(code: identity.code, identity: identity), on: NWEndpoint.Port(rawValue: port)!)
        listener.newConnectionHandler = accepted
        listener.stateUpdateHandler = { state in
            switch state {
            case .ready: status("等待 A 端连接 · UDP \(port)")
            case .failed(let error): status("监听失败：\(error.localizedDescription)")
            default: break
            }
        }
        listener.start(queue: queue)
    }
    public func stop() { listener.cancel() }
}
