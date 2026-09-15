import SwiftUI
import AppKit
import CoreImage.CIFilterBuiltins

/// Read-only projection of the Rust service. No service preferences or task history live in Swift.
private struct LanWebProjection: Decodable {
    let enabled: Bool
    let running: Bool
    let address: String?
    let preferredInterface: String?
    let interfaces: [NetworkInterface]
    let error: String?
    let tasks: [UploadTask]

    struct NetworkInterface: Decodable, Identifiable {
        let name: String
        let address: String
        let label: String
        var id: String { name }
    }

    struct UploadTask: Decodable, Identifiable {
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

struct LanTransferView: View {
    let store: AppStore
    @State private var state: LanWebProjection?
    @State private var configuring = false
    @State private var requestGeneration = 0
    @State private var readError: String?
    @State private var actionError: String?
    @State private var addressCopied = false
    @State private var qrImage: NSImage?

    private var liveAddress: String? {
        guard readError == nil, !configuring, state?.running == true else { return nil }
        return state?.address
    }

    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("局域网传输").font(.system(size: 26, weight: .bold))
                        Text("手机连接同一局域网，扫码发送照片和文字，或取用剪贴板列表。")
                            .font(.callout).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    serviceCard
                    if let message = actionError ?? readError ?? state?.error {
                        Label(message, systemImage: "exclamationmark.triangle")
                            .font(.callout).foregroundStyle(.orange)
                            .fixedSize(horizontal: false, vertical: true)
                            .textSelection(.enabled)
                    }
                    accessCard(compact: geometry.size.width < 650)
                    tasksCard
                }
                .padding(28)
                .frame(maxWidth: .infinity, alignment: .topLeading)
            }
            .background(Color(nsColor: .windowBackgroundColor))
        }
        .task {
            await refresh()
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { break }
                await refresh()
            }
        }
        .onChange(of: liveAddress, initial: true) { _, address in
            qrImage = address.flatMap(makeQRCode)
            addressCopied = false
        }
    }

    private var serviceCard: some View {
        card {
            HStack(spacing: 16) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("允许局域网访问").font(.headline)
                    Text(statusText).font(.callout).foregroundStyle(.secondary)
                }
                Spacer(minLength: 10)
                if configuring { ProgressView().controlSize(.small) }
                Toggle("允许局域网访问", isOn: Binding(
                    get: { state?.enabled ?? false },
                    set: { configure(enabled: $0, interface: state?.preferredInterface) }
                ))
                .labelsHidden().toggleStyle(.switch)
                .disabled(configuring || state == nil)
            }
            Text("开启后，同一局域网内的设备无需配对即可访问列表。离开此页面仍保持服务，重启后自动恢复开关。")
                .font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let state {
                Divider()
                Picker("网络接口", selection: Binding(
                    get: { state.preferredInterface ?? "" },
                    set: { configure(enabled: state.enabled, interface: $0.isEmpty ? nil : $0) }
                )) {
                    Text("自动选择").tag("")
                    ForEach(state.interfaces) { interface in
                        Text("\(interface.label) · \(interface.address)").tag(interface.name)
                    }
                    if let preferred = state.preferredInterface,
                       !state.interfaces.contains(where: { $0.name == preferred }) {
                        Text("\(preferred)（当前不可用）").tag(preferred)
                    }
                }
                .pickerStyle(.menu).disabled(configuring)
                .frame(maxWidth: 480, alignment: .leading)
            }
        }
    }

    private var statusText: String {
        if configuring { return "正在应用设置…" }
        if readError != nil { return "暂时无法读取服务状态" }
        guard let state else { return "正在读取服务状态…" }
        if !state.enabled { return "已关闭" }
        if state.running { return "服务运行中" }
        return state.error == nil ? "正在等待可用的局域网地址" : "服务未能运行"
    }

    private func accessCard(compact: Bool) -> some View {
        card {
            if let address = liveAddress {
                if compact {
                    VStack(alignment: .leading, spacing: 20) {
                        qrCode.frame(maxWidth: .infinity)
                        addressDetails(address)
                    }
                } else {
                    HStack(alignment: .center, spacing: 28) {
                        qrCode
                        addressDetails(address)
                    }
                }
            } else {
                ContentUnavailableView(
                    state?.enabled == true ? "等待可访问地址" : "开启后扫码传输",
                    systemImage: "qrcode",
                    description: Text(state?.enabled == true
                        ? "服务成功监听局域网后，二维码和地址将在这里显示。"
                        : "照片和文字会保存至共用剪贴板列表，网页也可浏览和下载已有内容。")
                ).frame(maxWidth: .infinity).padding(.vertical, 8)
            }
        }
    }

    @ViewBuilder private var qrCode: some View {
        if let qrImage {
            Image(nsImage: qrImage)
                .interpolation(.none).resizable().scaledToFit()
                .frame(width: 184, height: 184).padding(16)
                .background(Color.white, in: RoundedRectangle(cornerRadius: 12))
                .accessibilityLabel("用手机相机扫描二维码打开局域网传输网页")
        } else {
            ContentUnavailableView("二维码生成失败", systemImage: "qrcode", description: Text("仍可使用右侧地址访问。"))
                .frame(width: 216, height: 216)
        }
    }

    private func addressDetails(_ address: String) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            Label("使用手机相机扫码", systemImage: "iphone").font(.headline)
            Text("发送文字、上传照片，或浏览并取用 Mac 上的列表内容。")
                .font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Text(address).font(.system(.callout, design: .monospaced))
                .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            NativeGlassGroup {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 10) { addressButtons(address) }
                    VStack(alignment: .leading, spacing: 10) { addressButtons(address) }
                }
            }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private func addressButtons(_ address: String) -> some View {
        WorkspaceChromeButton(addressCopied ? "已复制地址" : "复制地址", systemImage: "doc.on.doc") {
            NSPasteboard.general.clearContents()
            addressCopied = NSPasteboard.general.setString(address, forType: .string)
            if !addressCopied { actionError = "未能写入系统剪贴板，请选择地址后手动复制。" }
        }
        WorkspaceChromeButton("打开网页", systemImage: "arrow.up.right.square") {
            guard let url = URL(string: address), NSWorkspace.shared.open(url) else {
                actionError = "无法打开浏览器，请复制地址后手动访问。"
                return
            }
        }
    }

    private var tasksCard: some View {
        card {
            HStack {
                Text("本次运行的接收任务").font(.headline)
                Spacer()
                if let state { Text("\(state.tasks.count)").foregroundStyle(.secondary).monospacedDigit() }
            }
            if let state, !state.tasks.isEmpty {
                LazyVStack(alignment: .leading, spacing: 16) {
                    ForEach(state.tasks.reversed()) { task in
                        Divider()
                        taskRow(task)
                    }
                }
            } else {
                ContentUnavailableView("尚无接收任务", systemImage: "tray.and.arrow.down",
                                       description: Text("手机发送照片或文字后，会在这里显示接收进度和保存结果。"))
                    .frame(maxWidth: .infinity)
            }
        }
    }

    private func taskRow(_ task: LanWebProjection.UploadTask) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 12) {
                Text(task.name).font(.callout.weight(.semibold)).lineLimit(2).truncationMode(.middle)
                Spacer(minLength: 8)
                Text(task.title).font(.caption).foregroundStyle(task.state == "error" ? Color.orange : .secondary)
            }
            if task.active {
                if task.state == "receiving", let total = task.totalBytes, total > 0 {
                    ProgressView(value: Double(min(task.receivedBytes, total)), total: Double(total))
                    Text("\(byteCount(task.receivedBytes)) / \(byteCount(total))")
                        .font(.caption).foregroundStyle(.secondary).monospacedDigit()
                } else {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text(task.state == "committing" ? "正在保存到列表…" : "已接收 \(byteCount(task.receivedBytes))")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            if !task.message.isEmpty {
                Text(task.message).font(.callout).foregroundStyle(.secondary)
                    .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
            }
            if let written = task.clipboardWritten {
                Label(written ? "已写入 Mac 剪贴板" : "未覆盖 Mac 当前剪贴板",
                      systemImage: written ? "checkmark.circle" : "clipboard")
                    .font(.caption).foregroundStyle(written ? Color.green : .secondary)
            }
        }
    }

    private func card<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 16, content: content)
            .padding(20).frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 16))
            .overlay { RoundedRectangle(cornerRadius: 16).strokeBorder(.quaternary, lineWidth: 1) }
    }

    @MainActor private func refresh() async {
        guard !configuring, let client = store.client else { return }
        let generation = requestGeneration
        do {
            let projection = try LanWebProjection.decode(await client.lanWebState())
            guard !Task.isCancelled, !configuring, generation == requestGeneration else { return }
            state = projection
            readError = nil
        } catch {
            guard !Task.isCancelled, !configuring, generation == requestGeneration else { return }
            readError = error.userFacingMessage
        }
    }

    @MainActor private func configure(enabled: Bool, interface: String?) {
        guard !configuring, let client = store.client else { return }
        configuring = true
        requestGeneration += 1
        actionError = nil
        Task {
            defer { configuring = false }
            do {
                state = try LanWebProjection.decode(await client.configureLanWeb(enabled: enabled, interfaceName: interface))
                readError = nil
            } catch {
                actionError = error.userFacingMessage
                // Do not keep advertising an old listener after an uncertain configuration result.
                readError = "正在重新确认服务状态"
            }
        }
    }

    private func makeQRCode(_ address: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(address.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)),
              let image = CIContext().createCGImage(output, from: output.extent) else { return nil }
        return NSImage(cgImage: image, size: NSSize(width: CGFloat(image.width), height: CGFloat(image.height)))
    }
}
