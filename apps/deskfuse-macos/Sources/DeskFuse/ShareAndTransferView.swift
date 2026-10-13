import SwiftUI
import AppKit
import CoreImage.CIFilterBuiltins
import DeskFuseEngine

/// Read-only projection of the Rust service. No service preferences or task history live in Swift.
private struct LanWebProjection: Decodable {
    let enabled: Bool
    let running: Bool
    let address: String?
    let addresses: [String]
    let listenAddress: String?
    let port: UInt16
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

/// One scrolling page: LAN sharing service on top, global transfer queue below.
struct ShareAndTransferView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    @State private var state: LanWebProjection?
    @State private var configuring = false
    @State private var requestGeneration = 0
    @State private var readError: String?
    @State private var actionError: String?
    @State private var addressCopied = false
    @State private var qrImage: NSImage?
    @State private var portDraft = ""
    @State private var selectedAddress = ""

    private var liveAddress: String? {
        guard readError == nil, !configuring, state?.running == true else { return nil }
        if state?.addresses.contains(selectedAddress) == true { return selectedAddress }
        return state?.address
    }

    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    SystemSettingsPageHeader("分享与传输", subtitle: "局域网内扫码分享剪贴板列表，或查看设备之间的文件传输。") {
                        WorkspaceChromeButton("暂存区", systemImage: "tray", help: "打开文件暂存区（⇧⌘D）") {
                            store.shelf?.show()
                        }
                    }
                    if let message = actionError ?? readError ?? state?.error {
                        Label(message, systemImage: "exclamationmark.triangle")
                            .font(.callout).foregroundStyle(.orange)
                            .fixedSize(horizontal: false, vertical: true)
                            .textSelection(.enabled)
                    }
                    shareCard(compact: geometry.size.width < 650)
                    receiveTasksCard
                    transfersSection(compact: geometry.size.width < 860)
                }
                .padding(.horizontal, SystemSettingsMetrics.pageHorizontalInset)
                .padding(.vertical, SystemSettingsMetrics.pageTopInset)
                .frame(maxWidth: .infinity, minHeight: geometry.size.height, alignment: .topLeading)
            }
            .background(.clear)
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
        .onChange(of: state?.port, initial: true) { _, port in
            portDraft = port.flatMap { $0 == 0 ? nil : String($0) } ?? ""
        }
    }

    private func shareCard(compact: Bool) -> some View {
        card {
            HStack(alignment: .top, spacing: 16) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("局域网扫码传输").font(.headline)
                    Text("在同一局域网内，其他设备可通过扫描二维码向这台设备发送文件。")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 10)
                if configuring { ProgressView().controlSize(.small) }
                Toggle("允许局域网访问", isOn: Binding(
                    get: { state?.enabled ?? false },
                    set: { configure(enabled: $0, interface: state?.preferredInterface) }
                ))
                .labelsHidden()
                .toggleStyle(.switch)
                .disabled(configuring || state == nil)
            }
            Divider()
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
                        : "开启右上角开关后，二维码和访问地址会显示在这里。")
                )
                .frame(maxWidth: .infinity)
                .padding(.vertical, 8)
            }
            if let state {
                DisclosureGroup("网络设置") {
                    VStack(alignment: .leading, spacing: 12) {
                        Picker("网络接口", selection: Binding(
                            get: { state.preferredInterface ?? "" },
                            set: { configure(enabled: state.enabled, interface: $0.isEmpty ? nil : $0) }
                        )) {
                            Text("所有接口（0.0.0.0）").tag("")
                            ForEach(state.interfaces) { interface in Text(interface.label).tag(interface.name) }
                        }
                        .pickerStyle(.menu)
                        HStack(spacing: 12) {
                            Text("端口")
                            TextField("自动分配", text: $portDraft)
                                .textFieldStyle(.roundedBorder)
                                .frame(width: 130)
                                .onSubmit { applyPort() }
                            Button("应用") { applyPort() }
                            Button("自动分配") {
                                portDraft = ""
                                configure(enabled: state.enabled, interface: state.preferredInterface, port: 0)
                            }
                        }
                        Text("留空由系统随机分配可用端口，也可指定 1–65535。")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.top, 12)
                }
            }
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
                    Text("所有接口（0.0.0.0）").tag("")
                    ForEach(state.interfaces) { interface in
                        Text(interface.label).tag(interface.name)
                    }
                    if let preferred = state.preferredInterface,
                       !state.interfaces.contains(where: { $0.name == preferred }) {
                        Text("\(preferred)（当前不可用）").tag(preferred)
                    }
                }
                .pickerStyle(.menu).disabled(configuring)
                .frame(maxWidth: 480, alignment: .leading)
                HStack(spacing: 12) {
                    Text("端口")
                    TextField("自动分配", text: $portDraft)
                        .textFieldStyle(.roundedBorder).frame(width: 130)
                        .onSubmit { applyPort() }
                    Button("应用") { applyPort() }
                        .disabled(configuring)
                    Button("自动分配") {
                        portDraft = ""
                        configure(enabled: state.enabled, interface: state.preferredInterface, port: 0)
                    }.disabled(configuring)
                }.disabled(configuring)
                Text("留空由系统随机分配可用端口，也可指定 1–65535。")
                    .font(.caption).foregroundStyle(.secondary)
                if state.running, let listenAddress = state.listenAddress {
                    Text("正在监听：\(listenAddress)")
                        .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                }
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
            if let state, state.addresses.count > 1 {
                Picker("访问地址", selection: Binding(
                    get: { liveAddress ?? "" },
                    set: { selectedAddress = $0 }
                )) {
                    ForEach(state.addresses, id: \.self) { address in
                        Text(address).tag(address)
                    }
                }.pickerStyle(.menu)
            }
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

    private var receiveTasksCard: some View {
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
                        receiveTaskRow(task)
                    }
                }
            } else {
                ContentUnavailableView("尚无接收任务", systemImage: "tray.and.arrow.down",
                                       description: Text("手机发送照片或文字后，会在这里显示接收进度和保存结果。"))
                    .frame(maxWidth: .infinity)
            }
        }
    }

    private func receiveTaskRow(_ task: LanWebProjection.UploadTask) -> some View {
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

    private func transfersSection(compact: Bool) -> some View {
        card {
            VStack(alignment: .leading, spacing: 6) {
                Text("传输任务").font(.headline)
                Text("文件在目标设备粘贴时传输，也可以从暂存区立即发送。")
                    .font(.callout).foregroundStyle(.secondary)
            }
            HStack(spacing: 0) {
                summary("进行中", count: snapshot.transfers.filter { [.sending, .receiving, .verifying, .delivering].contains($0.phase) }.count, color: .accentColor)
                Divider().frame(height: 18)
                summary("排队", count: snapshot.transfers.filter { $0.phase == .queued }.count, color: .primary)
                Divider().frame(height: 18)
                summary("可重试", count: snapshot.transfers.filter { $0.canRetry }.count, color: .orange)
            }
            .padding(.vertical, 8)
            if !snapshot.transfers.isEmpty {
                LazyVStack(spacing: 0) {
                    ForEach(snapshot.transfers, id: \.id) { task in
                        TransferQueueRow(store: store, task: task, compact: compact)
                        if task.id != snapshot.transfers.last?.id { Divider() }
                    }
                }
            }
            if snapshot.transfers.isEmpty {
                ContentUnavailableView {
                    Label {
                        Text("没有传输任务")
                    } icon: {
                        Image(systemName: "arrow.up.arrow.down")
                            .foregroundStyle(DeskFuseBrand.fusionGradient)
                    }
                } description: {
                    Text("在目标设备粘贴文件时会自动开始传输。")
                }
                    .frame(maxWidth: .infinity).padding(.vertical, 24)
            }
        }
    }

    private func summary(_ title: String, count: Int, color: Color) -> some View {
        HStack(spacing: 12) {
            Text(title).font(.callout).foregroundStyle(.secondary)
            Text("\(count)").font(.system(size: 18, weight: .semibold)).monospacedDigit().foregroundStyle(color)
        }.frame(maxWidth: .infinity)
    }

    /// 分组 block：浅灰圆角承载一个功能区，替代原先只有内边距的伪卡片。
    private func card<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 16, content: content)
            .padding(20).frame(maxWidth: .infinity, alignment: .leading)
            .systemSettingsBlock()
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

    @MainActor private func applyPort() {
        let text = portDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let port = text.isEmpty ? UInt16(0) : UInt16(text) else {
            actionError = "请输入 1–65535 的端口号，或留空自动分配。"
            return
        }
        configure(enabled: state?.enabled ?? false, interface: state?.preferredInterface, port: port)
    }

    @MainActor private func configure(enabled: Bool, interface: String?, port: UInt16? = nil) {
        guard !configuring, let client = store.client else { return }
        let requestedPort = port ?? state?.port ?? 0
        configuring = true
        requestGeneration += 1
        actionError = nil
        Task {
            defer { configuring = false }
            do {
                state = try LanWebProjection.decode(await client.configureLanWeb(enabled: enabled, interfaceName: interface, port: requestedPort))
                portDraft = state?.port == 0 ? "" : String(state?.port ?? 0)
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

private struct TransferQueueRow: View {
    @Bindable var store: AppStore
    let task: Transfer
    let compact: Bool
    private var active: Bool { [.sending, .receiving, .verifying, .delivering].contains(task.phase) }
    private var color: Color { task.phase == .failed ? .orange : task.outgoing ? .accentColor : .green }

    var body: some View {
        HStack(alignment: .center, spacing: 20) {
            Image(systemName: task.outgoing ? "arrow.up" : "arrow.down")
                .font(.system(size: 24, weight: .medium)).foregroundStyle(color)
                .frame(width: 46, height: 46)
                .background(color.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
                .accessibilityLabel(task.outgoing ? "发送" : "接收")
            VStack(alignment: .leading, spacing: 8) {
                Text(task.name).font(.system(size: 15, weight: .semibold)).lineLimit(2).truncationMode(.middle)
                Text(task.detail).font(.callout).foregroundStyle(task.phase == .failed ? Color.orange : .secondary)
                if compact { progress }
            }.frame(maxWidth: .infinity, alignment: .leading)
            if !compact { progress.frame(width: 280, alignment: .leading) }
            Group {
                if task.phase == .failed || task.phase == .cancelled {
                    if task.canRetry { Button("重试") { store.send(.retryTransfer(id: task.id)) } }
                } else if task.phase != .completed {
                    Button("取消") { store.send(.cancelTransfer(id: task.id)) }
                }
            }.controlSize(.large).frame(width: 76)
        }.padding(.vertical, 22)
    }

    private var progressBytes: UInt64 { task.phase == .delivering ? task.saved : task.transferred }
    private var progressTotal: UInt64 { task.phase == .delivering ? task.saveTotal : task.total }

    @ViewBuilder private var progress: some View {
        if task.phase == .verifying {
            HStack(spacing: 10) {
                ProgressView().controlSize(.small)
                Text(task.phaseTitle).font(.caption).foregroundStyle(.secondary)
            }
        } else if active {
            VStack(alignment: .leading, spacing: 8) {
                if progressTotal > 0 {
                    HStack {
                        ProgressView(value: Double(min(progressBytes, progressTotal)), total: Double(progressTotal))
                        Text("\(Int(min(1, Double(progressBytes) / Double(progressTotal)) * 100))%")
                            .font(.caption).monospacedDigit().foregroundStyle(.secondary)
                    }
                    Text("\(byteCount(progressBytes)) / \(byteCount(progressTotal))")
                        .font(.caption).monospacedDigit().foregroundStyle(.secondary)
                } else {
                    HStack { ProgressView().controlSize(.small); Text("已传输 \(byteCount(progressBytes))") }
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        } else if task.phase == .queued {
            HStack(spacing: 10) { ProgressView().controlSize(.small); Text(task.phaseTitle).font(.caption).foregroundStyle(.secondary) }
        } else if task.phase == .completed {
            Label("已完成", systemImage: "checkmark.circle").font(.caption).foregroundStyle(.secondary)
        } else if task.phase == .cancelled {
            Text("已取消").font(.caption).foregroundStyle(.secondary)
        }
    }
}
