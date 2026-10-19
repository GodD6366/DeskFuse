import SwiftUI
import AppKit
import CoreImage.CIFilterBuiltins
import UniformTypeIdentifiers
import DeskFuseEngine

/// 只读投影：服务状态与接收任务都由 Rust 侧持有，Swift 不保存偏好或历史。
private struct LanWebProjection: Decodable {
    let enabled: Bool
    let running: Bool
    let address: String?
    let addresses: [String]
    let listenAddress: String?
    let port: UInt16
    let preferredInterface: String?
    let error: String?
    let tasks: [UploadTask]

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

/// 文件传输：发送区、传输队列、手机扫码三块，共用引擎的同一份暂存与传输状态。
struct FileTransferView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot

    @State private var state: LanWebProjection?
    @State private var configuring = false
    @State private var requestGeneration = 0
    @State private var readError: String?
    @State private var actionError: String?
    @State private var addressCopied = false
    @State private var qrImage: NSImage?
    @State private var selectedAddress = ""
    @State private var stagedFiles: [URL] = []
    @State private var targetPeer: String = ""
    @State private var dropTargeted = false

    private var connectedPeers: [Device] {
        snapshot.devices.filter { $0.member && $0.connected && $0.trusted && !$0.blocked }
    }

    private var liveAddress: String? {
        guard readError == nil, !configuring, state?.running == true else { return nil }
        if state?.addresses.contains(selectedAddress) == true { return selectedAddress }
        return state?.address
    }

    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(alignment: .leading, spacing: DFSpace.xl) {
                    if let message = actionError ?? readError ?? state?.error {
                        DFNotice(tone: .warning, title: "文件传输", message: message)
                    }
                    sendSection
                    transferSection(compact: geometry.size.width < 860)
                    shareSection(compact: geometry.size.width < 650)
                    if let state, !state.tasks.isEmpty {
                        receiveTaskSection(state)
                    }
                }
                .padding(.horizontal, DFMetrics.pageInset)
                .padding(.top, DFMetrics.pageTopInset)
                .padding(.bottom, DFMetrics.pageBottomInset)
                .frame(maxWidth: .infinity, minHeight: geometry.size.height, alignment: .topLeading)
            }
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
        .onChange(of: connectedPeers.map(\.id), initial: true) { _, ids in
            if !ids.contains(targetPeer) { targetPeer = ids.first ?? "" }
        }
    }

    // MARK: - 发送区

    private var sendSection: some View {
        DFSection("发送文件", footer: "也可以把文件拖到菜单栏图标或按 ⇧⌘D 呼出暂存区。") {
            VStack(alignment: .leading, spacing: DFSpace.m) {
                HStack(spacing: DFSpace.m) {
                    Image(systemName: "tray.and.arrow.up")
                        .font(DFType.icon(.large))
                        .dfSecondaryText()
                    VStack(alignment: .leading, spacing: DFSpace.xxs) {
                        Text(stagedFiles.isEmpty ? "拖入文件，或选择要发送的内容" : "已选择 \(stagedFiles.count) 个文件")
                            .font(DFType.bodyMedium)
                        Text(stagedFiles.isEmpty
                             ? "文件只会发送给你选中的、当前在线的电脑。"
                             : byteCount(stagedFiles.reduce(0) { $0 + fileSize($1) }))
                            .font(DFType.detail)
                            .dfSecondaryText()
                    }
                    Spacer(minLength: DFSpace.m)
                    Button("选择文件…", action: chooseFiles)
                        .buttonStyle(.dfSecondary)
                }
                .padding(DFSpace.l)
                .frame(maxWidth: .infinity, alignment: .leading)
                .dfControlSurface(radius: DFRadius.card)
                .overlay {
                    RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous)
                        .strokeBorder(dropTargeted ? DFPalette.accent : .clear, lineWidth: 1.5)
                }
                .onDrop(of: [.fileURL], isTargeted: $dropTargeted) { providers in
                    loadFiles(from: providers)
                    return true
                }
                HStack(spacing: DFSpace.m) {
                    if connectedPeers.isEmpty {
                        Text("还没有已连接的电脑；先到设备页添加。")
                            .font(DFType.detail)
                            .dfSecondaryText()
                    } else {
                        Picker("发送到", selection: $targetPeer) {
                            ForEach(connectedPeers, id: \.id) { device in
                                Text(device.name).tag(device.id)
                            }
                        }
                        .pickerStyle(.menu)
                        .frame(maxWidth: 260)
                    }
                    Spacer(minLength: DFSpace.s)
                    if !stagedFiles.isEmpty {
                        Button("加入暂存区") {
                            store.shelf?.append(stagedFiles)
                            stagedFiles = []
                        }
                        .buttonStyle(.dfPlain)
                    }
                    Button("发送") { send() }
                        .buttonStyle(.dfPrimary)
                        .disabled(stagedFiles.isEmpty || targetPeer.isEmpty)
                }
            }
        }
    }

    private func chooseFiles() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = true
        panel.prompt = "选择"
        panel.message = "选择要发送到另一台电脑的文件"
        guard panel.runModal() == .OK else { return }
        stagedFiles.append(contentsOf: panel.urls)
    }

    private func loadFiles(from providers: [NSItemProvider]) {
        for provider in providers {
            guard provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) else { continue }
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                Task { @MainActor in
                    if !stagedFiles.contains(url) { stagedFiles.append(url) }
                }
            }
        }
    }

    private func send() {
        guard !stagedFiles.isEmpty, !targetPeer.isEmpty else { return }
        store.send(.sendFiles(peerId: targetPeer, paths: stagedFiles.map(\.path)))
        stagedFiles = []
    }

    private func fileSize(_ url: URL) -> UInt64 {
        let values = try? url.resourceValues(forKeys: [.fileSizeKey, .totalFileAllocatedSizeKey])
        return UInt64(values?.fileSize ?? values?.totalFileAllocatedSize ?? 0)
    }

    // MARK: - 传输队列

    private func transferSection(compact: Bool) -> some View {
        DFSection("传输队列",
                  footer: snapshot.transfers.isEmpty ? "在另一台电脑粘贴文件时会自动开始传输，也可以从暂存区立即发送。" : nil) {
            if snapshot.transfers.isEmpty {
                DFEmptyState(symbol: "arrow.up.arrow.down",
                             title: "还没有传输任务",
                             message: "选择文件并发送后，进度会显示在这里。",
                             actionTitle: "选择文件发送",
                             action: chooseFiles)
                    .frame(maxWidth: .infinity)
            } else {
                VStack(spacing: 0) {
                    ForEach(snapshot.transfers, id: \.id) { task in
                        TransferQueueRow(store: store, task: task, compact: compact)
                        if task.id != snapshot.transfers.last?.id { Divider() }
                    }
                }
            }
        }
    }

    // MARK: - 手机扫码

    private func shareSection(compact: Bool) -> some View {
        DFSection("手机扫码传输", footer: "离开此页仍保持可用；重启后自动恢复上次的开关。") {
            HStack(alignment: .center, spacing: DFSpace.m) {
                VStack(alignment: .leading, spacing: DFSpace.xxs) {
                    Text("允许局域网访问").font(DFType.bodyMedium)
                    Text("同一局域网内的手机扫码即可发送照片或文字。")
                        .font(DFType.detail)
                        .dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if configuring { ProgressView().controlSize(.small) }
                Toggle("允许局域网访问", isOn: Binding(
                    get: { state?.enabled ?? false },
                    set: { configure(enabled: $0) }
                ))
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.small)
                .disabled(configuring || state == nil)
            }
            Divider()
            if let address = liveAddress {
                if compact {
                    VStack(alignment: .leading, spacing: DFSpace.l) {
                        qrCode.frame(maxWidth: .infinity)
                        addressDetails(address)
                    }
                } else {
                    HStack(alignment: .center, spacing: DFSpace.xl) {
                        qrCode
                        addressDetails(address)
                    }
                }
            } else {
                DFEmptyState(symbol: "qrcode",
                             title: state?.enabled == true ? "等待可访问地址" : "开启后扫码传输",
                             message: state?.enabled == true
                                ? "服务成功监听局域网后，二维码和地址会显示在这里。"
                                : "开启开关后，二维码和访问地址会显示在这里。",
                             actionTitle: state?.enabled == true ? nil : "开启",
                             action: state?.enabled == true ? nil : { configure(enabled: true) })
                    .frame(maxWidth: .infinity)
            }
        }
    }

    @ViewBuilder private var qrCode: some View {
        if let qrImage {
            Image(nsImage: qrImage)
                .interpolation(.none).resizable().scaledToFit()
                .frame(width: 176, height: 176).padding(DFSpace.m)
                .background(Color.white, in: RoundedRectangle(cornerRadius: DFRadius.card))
                .accessibilityLabel("用手机相机扫描二维码打开局域网传输网页")
        } else {
            DFEmptyState(symbol: "qrcode", title: "二维码不可用",
                         message: "仍可使用右侧地址访问。")
                .frame(width: 208, height: 208)
        }
    }

    private func addressDetails(_ address: String) -> some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            Text("用手机相机扫码")
                .font(DFType.bodyMedium)
            Text("发送文字、上传照片，或浏览并取用这台 Mac 上的列表内容。")
                .font(DFType.detail)
                .dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
            Text(address)
                .font(DFType.mono)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if let state, state.addresses.count > 1 {
                Picker("访问地址", selection: Binding(
                    get: { liveAddress ?? "" },
                    set: { selectedAddress = $0 }
                )) {
                    ForEach(state.addresses, id: \.self) { Text($0).tag($0) }
                }
                .pickerStyle(.menu)
                .frame(maxWidth: 320)
            }
            HStack(spacing: DFSpace.s) {
                Button(addressCopied ? "已复制地址" : "复制地址", systemImage: "doc.on.doc") {
                    NSPasteboard.general.clearContents()
                    addressCopied = NSPasteboard.general.setString(address, forType: .string)
                    if !addressCopied { actionError = "未能写入系统剪贴板，请选择地址后手动复制。" }
                }
                .buttonStyle(.dfSecondary)
                Button("打开网页", systemImage: "arrow.up.right.square") {
                    guard let url = URL(string: address), NSWorkspace.shared.open(url) else {
                        actionError = "无法打开浏览器，请复制地址后手动访问。"
                        return
                    }
                }
                .buttonStyle(.dfSecondary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: - 手机接收任务

    private func receiveTaskSection(_ state: LanWebProjection) -> some View {
        DFSection("手机发送记录") {
            VStack(spacing: 0) {
                ForEach(state.tasks.reversed()) { task in
                    receiveTaskRow(task)
                    if task.id != state.tasks.first?.id { Divider() }
                }
            }
        }
    }

    private func receiveTaskRow(_ task: LanWebProjection.UploadTask) -> some View {
        VStack(alignment: .leading, spacing: DFSpace.xs) {
            HStack(alignment: .top, spacing: DFSpace.m) {
                Text(task.name)
                    .font(DFType.bodyMedium)
                    .lineLimit(2)
                    .truncationMode(.middle)
                Spacer(minLength: DFSpace.s)
                Text(task.title)
                    .font(DFType.detail)
                    .foregroundStyle(task.state == "error" ? DFPalette.warning : Color.secondary)
            }
            if task.active {
                if task.state == "receiving", let total = task.totalBytes, total > 0 {
                    ProgressView(value: Double(min(task.receivedBytes, total)), total: Double(total))
                    Text("\(byteCount(task.receivedBytes)) / \(byteCount(total))")
                        .font(DFType.detail).dfSecondaryText().monospacedDigit()
                } else {
                    HStack(spacing: DFSpace.s) {
                        ProgressView().controlSize(.small)
                        Text(task.state == "committing" ? "正在保存到列表…" : "已接收 \(byteCount(task.receivedBytes))")
                            .font(DFType.detail).dfSecondaryText()
                    }
                }
            }
            if !task.message.isEmpty {
                Text(task.message)
                    .font(DFType.detail)
                    .dfSecondaryText()
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let written = task.clipboardWritten {
                Label(written ? "已写入 Mac 剪贴板" : "未覆盖 Mac 当前剪贴板",
                      systemImage: written ? "checkmark.circle" : "clipboard")
                    .font(DFType.detail)
                    .foregroundStyle(written ? DFPalette.success : Color.secondary)
            }
        }
        .padding(.vertical, DFSpace.xs)
    }

    // MARK: - 引擎交互

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

    /// 端口与网络接口不再暴露给用户：始终由系统自动分配，沿用上次的接口选择。
    @MainActor private func configure(enabled: Bool) {
        guard !configuring, let client = store.client else { return }
        configuring = true
        requestGeneration += 1
        actionError = nil
        Task {
            defer { configuring = false }
            do {
                state = try LanWebProjection.decode(await client.configureLanWeb(
                    enabled: enabled,
                    interfaceName: state?.preferredInterface,
                    port: 0))
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
    private var color: Color { task.phase == .failed ? DFPalette.warning : (task.outgoing ? DFPalette.accent : DFPalette.success) }

    var body: some View {
        HStack(alignment: .center, spacing: DFSpace.l) {
            Image(systemName: task.outgoing ? "arrow.up" : "arrow.down")
                .font(DFType.icon(.large, weight: .medium))
                .foregroundStyle(color)
                .frame(width: 40, height: 40)
                .background(color.opacity(DFOpacity.soft), in: RoundedRectangle(cornerRadius: DFRadius.control))
                .accessibilityLabel(task.outgoing ? "发送" : "接收")
            VStack(alignment: .leading, spacing: DFSpace.xs) {
                Text(task.name).font(DFType.bodyMedium).lineLimit(2).truncationMode(.middle)
                Text(task.detail)
                    .font(DFType.detail)
                    .dfSecondaryText()
                if compact { progress }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if !compact { progress.frame(width: 260, alignment: .leading) }
            Group {
                if task.phase == .failed || task.phase == .cancelled {
                    if task.canRetry { Button("重试") { store.send(.retryTransfer(id: task.id)) } }
                } else if task.phase != .completed {
                    Button("取消") { store.send(.cancelTransfer(id: task.id)) }
                }
            }
            .buttonStyle(.dfPlain)
            .controlSize(.small)
            .frame(width: 72)
        }
        .padding(.vertical, DFSpace.m)
    }

    private var progressBytes: UInt64 { task.phase == .delivering ? task.saved : task.transferred }
    private var progressTotal: UInt64 { task.phase == .delivering ? task.saveTotal : task.total }

    @ViewBuilder private var progress: some View {
        if task.phase == .verifying {
            HStack(spacing: DFSpace.s) {
                ProgressView().controlSize(.small)
                Text(task.phaseTitle).font(DFType.detail).dfSecondaryText()
            }
        } else if active {
            VStack(alignment: .leading, spacing: DFSpace.xs) {
                if progressTotal > 0 {
                    HStack(spacing: DFSpace.s) {
                        ProgressView(value: Double(min(progressBytes, progressTotal)), total: Double(progressTotal))
                        Text("\(Int(min(1, Double(progressBytes) / Double(progressTotal)) * 100))%")
                            .font(DFType.detail).dfSecondaryText().monospacedDigit()
                    }
                    Text("\(byteCount(progressBytes)) / \(byteCount(progressTotal))")
                        .font(DFType.detail).dfSecondaryText().monospacedDigit()
                } else {
                    HStack(spacing: DFSpace.s) {
                        ProgressView().controlSize(.small)
                        Text("已传输 \(byteCount(progressBytes))").font(DFType.detail).dfSecondaryText()
                    }
                }
            }
        } else if task.phase == .queued {
            HStack(spacing: DFSpace.s) {
                ProgressView().controlSize(.small)
                Text(task.phaseTitle).font(DFType.detail).dfSecondaryText()
            }
        } else if task.phase == .completed {
            Label("已完成", systemImage: "checkmark.circle").font(DFType.detail).dfSecondaryText()
        } else if task.phase == .cancelled {
            Text("已取消").font(DFType.detail).dfSecondaryText()
        }
    }
}

/// Shared phase wording for the transfer page and global progress panel.
extension Transfer {
    var phaseTitle: String {
        switch phase {
        case .queued: "准备中"
        case .sending: "发送中"
        case .receiving: "接收中"
        case .verifying: outgoing ? "等待校验" : "校验中"
        case .delivering: "保存中"
        case .completed: "已完成"
        case .cancelled: "已取消"
        case .failed: "失败"
        }
    }
}
