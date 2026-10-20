import AppKit
import CoreImage.CIFilterBuiltins
import Foundation
import Observation
import DeskFuseEngine

/// 只读投影：服务状态与接收任务都由 Rust 侧持有，Swift 不保存偏好或历史。
struct LanWebProjection: Decodable, Sendable {
    let enabled: Bool
    let running: Bool
    let address: String?
    let addresses: [String]
    let listenAddress: String?
    let port: UInt16
    let preferredInterface: String?
    let error: String?
    let tasks: [UploadTask]

    struct UploadTask: Decodable, Sendable, Identifiable {
        let id: String
        let name: String
        let state: String
        let receivedBytes: UInt64
        let totalBytes: UInt64?
        let clipboardWritten: Bool?
        let message: String

        var active: Bool { state == "receiving" || state == "committing" }
        var title: String {
            switch state {
            case "receiving": "正在接收"
            case "committing": "正在保存"
            case "done": "已保存"
            case "error": "接收失败"
            case "cancelled": "已取消"
            default: "等待状态更新"
            }
        }
    }

    static func decode(_ json: String) throws -> Self {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(Self.self, from: Data(json.utf8))
    }
}

/// 局域网共享的界面投影。页面切换会重建视图，上一次的状态留在这里，
/// 重新进入时直接用已知结果渲染：开关不会先回到关闭再跳开，二维码也不会重排。
@MainActor @Observable
final class LanWebSharing {
    static let shared = LanWebSharing()

    private(set) var state: LanWebProjection?
    private(set) var configuring = false
    /// 读取状态失败时的提示；读到有效状态后清空。
    private(set) var readError: String?
    var actionError: String?
    /// 多地址时记住用户的选择，回到页面不重置。
    var selectedAddress = ""
    private(set) var qrImage: NSImage?

    private var client: EngineClient?
    private var generation = 0

    /// 只展示已确认在监听、且读取没有失败的地址。
    var liveAddress: String? {
        guard readError == nil, let state, state.running else { return nil }
        if state.addresses.contains(selectedAddress) { return selectedAddress }
        return state.address
    }

    func attach(_ client: EngineClient?) {
        guard self.client !== client else { return }
        self.client = client
    }

    /// 启动后先取一次真实状态：首次打开页面也不必先渲染默认值再跳变。
    func warmUp(_ client: EngineClient) {
        attach(client)
        Task { await refresh() }
    }

    func refresh() async {
        guard !configuring, let client else { return }
        let generation = generation
        do {
            let projection = try LanWebProjection.decode(await client.lanWebState())
            guard !Task.isCancelled, !configuring, generation == self.generation else { return }
            readError = nil
            apply(projection)
        } catch {
            guard !Task.isCancelled, !configuring, generation == self.generation else { return }
            readError = error.userFacingMessage
        }
    }

    /// 端口与网络接口不再暴露给用户：始终由系统自动分配，沿用上次的接口选择。
    func configure(enabled: Bool) {
        guard !configuring, let client else { return }
        configuring = true
        generation += 1
        actionError = nil
        Task {
            defer { configuring = false }
            do {
                let projection = try LanWebProjection.decode(await client.configureLanWeb(
                    enabled: enabled,
                    interfaceName: state?.preferredInterface,
                    port: 0))
                readError = nil
                apply(projection)
            } catch {
                actionError = error.userFacingMessage
                // 结果不确定时不再继续展示旧的监听地址。
                readError = "正在重新确认服务状态"
            }
        }
    }

    private func apply(_ projection: LanWebProjection) {
        let previousAddress = liveAddress
        state = projection
        if !projection.addresses.contains(selectedAddress) {
            selectedAddress = projection.address ?? projection.addresses.first ?? ""
        }
        // 轮询每秒都会写入新投影，地址没变就不要重建位图，避免二维码跟着刷新闪动。
        let address = liveAddress
        if address != previousAddress || qrImage == nil { qrImage = address.flatMap(Self.makeQRCode) }
    }

    private static func makeQRCode(_ address: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(address.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)),
              let image = CIContext().createCGImage(output, from: output.extent) else { return nil }
        return NSImage(cgImage: image, size: NSSize(width: CGFloat(image.width), height: CGFloat(image.height)))
    }
}
