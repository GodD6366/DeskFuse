import Foundation
import CUniDeskRdp

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
    private let frameLock = NSLock()

    public var state: State {
        lock.withLock { _state }
    }

    public init() {}

    /// Starts a connection. `rdpFile` must have `full address` already resolved
    /// (the `{host}` token replaced with the peer IPv4 and `:3389`).
    public func start(rdpFile: String, username: String?, password: String?) {
        stop()
        lock.withLock { _state = .connecting }
        let selfPointer = Unmanaged.passUnretained(self).toOpaque()
        handle = rdp_bridge_create(
            rdpFile,
            username,
            password,
            RemoteRdpBridge.callback,
            selfPointer)
        guard handle != nil, rdp_bridge_start(handle) == 0 else {
            lock.withLock { _state = .failed("无法启动 RDP 会话") }
            return
        }
    }

    public func stop() {
        if let handle {
            rdp_bridge_stop(handle)
            rdp_bridge_free(handle)
            self.handle = nil
        }
        lock.withLock { _state = .idle }
    }

    public var connected: Bool {
        guard let handle else { return false }
        return rdp_bridge_connected(handle) != 0
    }

    /// Copies the latest frame into `destination` (BGRA / XRGB32 byte order).
    /// Returns the frame size on success, or nil when no frame is available.
    public func copyFrame(destination: UnsafeMutableRawPointer, stride: UInt32) -> (width: UInt32, height: UInt32)? {
        guard let handle else { return nil }
        var width: UInt32 = 0
        var height: UInt32 = 0
        guard rdp_bridge_frame_size(handle, &width, &height) == 0 else { return nil }
        guard rdp_bridge_copy_frame(handle, destination, stride) == 0 else { return nil }
        return (width, height)
    }

    public func mouse(x: UInt16, y: UInt16, flags: UInt16) {
        guard let handle else { return }
        rdp_bridge_mouse(handle, x, y, flags)
    }

    public func key(flags: UInt16, scancode: UInt8) {
        guard let handle else { return }
        rdp_bridge_key(handle, flags, scancode)
    }

    public func unicode(flags: UInt16, code: UInt16) {
        guard let handle else { return }
        rdp_bridge_unicode(handle, flags, code)
    }

    private func handle(event: Int, message: String?) {
        lock.withLock {
            switch event {
            case RDP_EVENT_CONNECTED:
                _state = .connected
            case RDP_EVENT_DISCONNECTED:
                _state = .disconnected
            case RDP_EVENT_AUTH_REQUIRED:
                _state = .connecting
            case RDP_EVENT_ERROR:
                _state = .failed(message ?? "RDP 连接失败")
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
        Int32, UnsafePointer<CChar>?, UnsafeMutableRawPointer?
    ) -> Void = { event, message, userdata in
        guard let userdata else { return }
        let bridge = Unmanaged<RemoteRdpBridge>.fromOpaque(userdata).takeUnretainedValue()
        bridge.handle(event: Int(event), message: message.map { String(cString: $0) })
    }
}
