import Foundation
import CDeskFuseRdp

/// Swift wrapper over the FreeRDP headless client bridge (`RdpBridge.c`).
///
/// One instance owns one RDP connection. The bridge runs FreeRDP's event loop on
/// its own thread; this wrapper polls the composed framebuffer and forwards
/// input. The credentials and the `.rdp` document come from the catalog protocol
/// (see `RemoteRdpCatalog`), never from this class.
@available(macOS 14.2, *)
public final class RemoteRdpBridge: @unchecked Sendable {
    public enum State: Equatable {
        case idle
        case connecting
        case connected
        case disconnected
        case failed(String)
    }

    private var handle: OpaquePointer?
    private let lock = NSLock()
    private var _state: State = .idle
    private var _certificateFingerprint: String?
    private let handleLock = NSRecursiveLock()

    public var state: State {
        lock.withLock { _state }
    }

    /// Fingerprint reported by the most recent rejected RDP certificate.
    /// It is exposed only so the UI can show it for an explicit trust choice.
    public var certificateFingerprint: String? {
        lock.withLock { _certificateFingerprint }
    }

    public init() {}

    /// Starts a connection. `rdpFile` must have `full address` already resolved
    /// (the `{host}` token replaced with the peer IPv4 and `:3389`).
    public func start(rdpFile: String, username: String?, password: String?, expectedCertificateFingerprint: String? = nil) {
        handleLock.lock(); defer { handleLock.unlock() }
        stop()
        lock.withLock { _state = .connecting; _certificateFingerprint = nil }
        let selfPointer = Unmanaged.passUnretained(self).toOpaque()
        handle = rdp_bridge_create(
            rdpFile,
            username,
            password,
            expectedCertificateFingerprint,
            RemoteRdpBridge.callback,
            selfPointer)
        guard handle != nil, rdp_bridge_start(handle) == 0 else {
            lock.withLock { _state = .failed("无法启动 RDP 会话") }
            return
        }
    }

    public func stop() {
        handleLock.lock(); defer { handleLock.unlock() }
        if let handle {
            rdp_bridge_stop(handle)
            rdp_bridge_free(handle)
            self.handle = nil
        }
        lock.withLock { _state = .idle }
    }

    public var connected: Bool {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle else { return false }
        return rdp_bridge_connected(handle) != 0
    }

    public struct Frame: Sendable {
        public let bytes: Data
        public let width: Int
        public let height: Int
    }

    /// Size is rechecked under the C frame mutex before copying: a resize
    /// between allocation and copy cannot overrun or mislabel the destination.
    public func frame() -> Frame? {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle else { return nil }
        var width: UInt32 = 0, height: UInt32 = 0
        guard rdp_bridge_frame_size(handle, &width, &height) == 0,
              width > 0, height > 0, width <= 8192, height <= 8192 else { return nil }
        let stride = width * 4
        var bytes = Data(count: Int(stride) * Int(height))
        let count = bytes.count
        let copied: Int32 = bytes.withUnsafeMutableBytes { raw in
            guard let base = raw.baseAddress else { return Int32(-1) }
            return rdp_bridge_copy_frame(handle, base, stride, count, width, height)
        }
        guard copied == 0 else { return nil }
        return Frame(bytes: bytes, width: Int(width), height: Int(height))
    }

    public func mouse(x: UInt16, y: UInt16, flags: UInt16) {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle else { return }
        rdp_bridge_mouse(handle, x, y, flags)
    }

    public func key(flags: UInt16, scancode: UInt8) {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle else { return }
        rdp_bridge_key(handle, flags, scancode)
    }

    public func unicode(flags: UInt16, code: UInt16) {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle else { return }
        rdp_bridge_unicode(handle, flags, code)
    }

    private func handle(event: Int, message: String?, fingerprint: String?) {
        lock.withLock {
            switch event {
            case RDP_EVENT_CONNECTED:
                _state = .connected
            case RDP_EVENT_DISCONNECTED:
                if case .failed = _state { break }
                _state = .disconnected
            case RDP_EVENT_AUTH_REQUIRED:
                _state = .connecting
            case RDP_EVENT_ERROR:
                _state = .failed(message ?? "RDP 连接失败")
            case RDP_EVENT_CERTIFICATE_REJECTED:
                _certificateFingerprint = fingerprint
				_state = .failed(message ?? "RDP 服务器证书未获信任，请核对指纹后选择信任")
            default:
                break
            }
        }
    }

    deinit {
        stop()
    }

    /// Non-capturing trampoline so Swift can hand a C function pointer to the
    /// bridge; the Swift object is recovered through `userdata`.
    private static let callback: @convention(c) (
        Int32, UnsafePointer<CChar>?, UnsafePointer<CChar>?, UnsafeMutableRawPointer?
    ) -> Void = { event, message, fingerprint, userdata in
        guard let userdata else { return }
        let bridge = Unmanaged<RemoteRdpBridge>.fromOpaque(userdata).takeUnretainedValue()
        bridge.handle(event: Int(event), message: message.map { String(cString: $0) },
                      fingerprint: fingerprint.map { String(cString: $0) })
    }
}
