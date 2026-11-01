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

    /// Clipboard traffic carried by the session's cliprdr channel.
    public enum ClipboardEvent: Sendable {
        case ready
        case formats([UInt32])
        case data(format: UInt32, data: Data?)
        case request(format: UInt32)
    }

    private var handle: OpaquePointer?
    private let lock = NSLock()
    private var _state: State = .idle
    private let handleLock = NSRecursiveLock()
    private let clipboardLock = NSLock()
    private var clipboardHandler: (@Sendable (ClipboardEvent) -> Void)?
    private let authLock = NSLock()
    private var authenticationFailed = false

    public var state: State {
        lock.withLock { _state }
    }

    public init() {}

    /// Creates the native session without connecting. The caller must transfer
    /// the tunnel descriptor to the engine and then call `start`.
    @discardableResult
    public func prepare(rdpFile: String, username: String?, password: String?,
                        expectedCertificateFingerprint: String) -> Bool {
        handleLock.lock(); defer { handleLock.unlock() }
        stop()
        authLock.withLock { authenticationFailed = false }
        lock.withLock { _state = .connecting }
        let selfPointer = Unmanaged.passUnretained(self).toOpaque()
        handle = rdp_bridge_create(
            rdpFile,
            username,
            password,
            expectedCertificateFingerprint,
            RemoteRdpBridge.callback,
            selfPointer)
        guard let handle else {
            lock.withLock { _state = .failed("无法创建 RDP 会话") }
            return false
        }
        rdp_bridge_set_clipboard_callback(handle, RemoteRdpBridge.clipboardCallback, selfPointer)
        return true
    }

    /// Installs the clipboard sink. `handler` runs on FreeRDP's event-loop
    /// thread and receives copied payloads; hop to your own actor there.
    public func setClipboardHandler(_ handler: (@Sendable (ClipboardEvent) -> Void)?) {
        clipboardLock.withLock { clipboardHandler = handler }
    }

    @discardableResult
    public func clipboardSetFormats(_ formats: [UInt32]) -> Bool {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle else { return false }
        let result = formats.withUnsafeBufferPointer { buffer in
            rdp_bridge_clipboard_set_formats(handle, buffer.baseAddress, buffer.count)
        }
        return result == 0
    }

    @discardableResult
    public func clipboardSetData(format: UInt32, data: Data) -> Bool {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle else { return false }
        let result: Int32 = data.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return -1 }
            return rdp_bridge_clipboard_set_data(handle, format, base, raw.count)
        }
        return result == 0
    }

    @discardableResult
    public func clipboardRequest(format: UInt32) -> Bool {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle else { return false }
        return rdp_bridge_clipboard_request(handle, format) == 0
    }

    /// True once after a credential rejection; the caller can prompt and retry.
    public func consumeAuthenticationFailure() -> Bool {
        authLock.withLock {
            let pending = authenticationFailed
            authenticationFailed = false
            return pending
        }
    }

    /// Must be called after `prepare` and before `start`; FreeRDP decides whether
    /// to load the rdpsnd channel while loading static addins.
    public func setAudioEnabled(_ enabled: Bool) {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle else { return }
        rdp_bridge_set_audio(handle, enabled ? 1 : 0)
    }

    /// Transfers the pump end of the bridge's socketpair exactly once. The
    /// returned descriptor is owned by the caller and must be closed or handed
    /// to `Engine.openRemoteRdpTunnel`.
    public func takeTunnelFD() -> Int32? {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle else { return nil }
        let fd = rdp_bridge_take_tunnel_fd(handle)
        return fd >= 0 ? fd : nil
    }

    /// Starts the prepared session. `rdpFile`'s logical host is never dialed;
    /// all bytes move over the engine-owned QUIC tunnel.
    public func start() {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle, rdp_bridge_start(handle) == 0 else {
            lock.withLock { _state = .failed("无法启动 RDP 会话") }
            return
        }
    }

    /// Launches an additional registered alias on the existing RAIL session.
    @discardableResult
    public func execute(_ name: String) -> Bool {
        handleLock.lock(); defer { handleLock.unlock() }
        guard let handle else { return false }
        return rdp_bridge_execute(handle, name) == 0
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

    private func handle(event: Int, message: String?) {
        lock.withLock {
            switch event {
            case RDP_EVENT_CONNECTED:
                _state = .connected
            case RDP_EVENT_DISCONNECTED:
                if case .failed = _state { break }
                _state = .disconnected
            case RDP_EVENT_AUTH_REQUIRED:
                _state = .connecting
            case RDP_EVENT_AUTH_FAILED:
                authLock.withLock { authenticationFailed = true }
                _state = .failed(message ?? "Windows 登录失败，请重新输入密码")
            case RDP_EVENT_ERROR:
                _state = .failed(message ?? "RDP 连接失败")
            case RDP_EVENT_CERTIFICATE_REJECTED:
                _state = .failed(message ?? "RDP 证书与已配对 Windows 的指纹不一致")
            default:
                break
            }
        }
    }

    deinit {
        stop()
    }

    private func handleClipboard(event: Int, format: UInt32, payload: Data?) {
        guard let handler = clipboardLock.withLock({ clipboardHandler }) else { return }
        switch event {
        case Int(RDP_CLIPBOARD_READY):
            handler(.ready)
        case Int(RDP_CLIPBOARD_FORMATS):
            guard let payload, payload.count >= 4, payload.count % 4 == 0 else { return }
            let bytes = [UInt8](payload)
            var ids: [UInt32] = []
            ids.reserveCapacity(bytes.count / 4)
            var index = 0
            while index + 3 < bytes.count {
                ids.append(UInt32(bytes[index]) |
                           (UInt32(bytes[index + 1]) << 8) |
                           (UInt32(bytes[index + 2]) << 16) |
                           (UInt32(bytes[index + 3]) << 24))
                index += 4
            }
            handler(.formats(ids))
        case Int(RDP_CLIPBOARD_DATA):
            handler(.data(format: format, data: payload))
        case Int(RDP_CLIPBOARD_REQUEST):
            handler(.request(format: format))
        default:
            break
        }
    }

    /// Non-capturing trampoline so Swift can hand a C function pointer to the
    /// bridge; the Swift object is recovered through `userdata`.
    private static let clipboardCallback: @convention(c) (
        Int32, UInt32, UnsafePointer<UInt8>?, Int, UnsafeMutableRawPointer?
    ) -> Void = { event, format, data, length, userdata in
        guard let userdata else { return }
        let bridge = Unmanaged<RemoteRdpBridge>.fromOpaque(userdata).takeUnretainedValue()
        var payload: Data?
        if let data, length > 0 {
            payload = Data(bytes: data, count: length)
        }
        bridge.handleClipboard(event: Int(event), format: format, payload: payload)
    }

    /// Non-capturing trampoline so Swift can hand a C function pointer to the
    /// bridge; the Swift object is recovered through `userdata`.
    private static let callback: @convention(c) (
        Int32, UnsafePointer<CChar>?, UnsafePointer<CChar>?, UnsafeMutableRawPointer?
    ) -> Void = { event, message, _, userdata in
        guard let userdata else { return }
        let bridge = Unmanaged<RemoteRdpBridge>.fromOpaque(userdata).takeUnretainedValue()
        bridge.handle(event: Int(event), message: message.map { String(cString: $0) })
    }
}
