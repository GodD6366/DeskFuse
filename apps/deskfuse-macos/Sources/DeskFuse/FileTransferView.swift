import SwiftUI
import AppKit
import UniformTypeIdentifiers
import DeskFuseEngine

/// 局域网传输：中列扫码与发送，右列任务，共用引擎的同一份暂存与传输状态。
struct FileTransferView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot

    /// 服务状态放在会话级投影里：切换页面重建视图时沿用上次结果，不再先渲染默认值再跳回。
    private var sharing: LanWebSharing { .shared }
    @State private var addressCopied = false
    @State private var stagedFiles: [URL] = []
    @State private var targetPeer: String = ""
    @State private var dropTargeted = false

    private var connectedPeers: [Device] {
        snapshot.devices.filter { $0.member && $0.connected && $0.trusted && !$0.blocked }
    }

    private var liveAddress: String? { sharing.liveAddress }

    var body: some View {
        GeometryReader { geometry in
            HStack(spacing: 0) {
                ScrollView {
                    VStack(alignment: .leading, spacing: DFSpace.l) {
                        if let message = sharing.actionError ?? sharing.readError ?? sharing.state?.error {
                            DFNotice(tone: .warning, title: "局域网传输", message: message)
                        }
                        shareSection
                        sendSection
                    }
                    .padding(DFSpace.l)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)

                Divider()

                ScrollView {
                    VStack(alignment: .leading, spacing: DFSpace.xl) {
                        transferSection(compact: true)
                        if let state = sharing.state, !state.tasks.isEmpty {
                            receiveTaskSection(state)
                        }
                    }
                    .padding(DFSpace.l)
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                }
                .frame(width: min(380, geometry.size.width * 0.38))
                .frame(maxHeight: .infinity)
            }
        }
        .task {
            sharing.attach(store.client)
            await sharing.refresh()
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { break }
                await sharing.refresh()
            }
        }
        .onChange(of: liveAddress) { _, _ in addressCopied = false }
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
                VStack(alignment: .leading, spacing: DFSpace.m) {
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
                    HStack(spacing: DFSpace.s) {
                        Button("选择文件…", action: chooseFiles)
                            .buttonStyle(.dfSecondary)
                        Spacer(minLength: 0)
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
        DFSection("传输任务",
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

    private var shareSection: some View {
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
                if sharing.configuring { ProgressView().controlSize(.small) }
                Toggle("允许局域网访问", isOn: Binding(
                    get: { sharing.state?.enabled ?? false },
                    set: { sharing.configure(enabled: $0) }
                ))
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.small)
                .disabled(sharing.configuring || sharing.state == nil)
            }
            Divider()
            if let address = liveAddress {
                HStack(alignment: .center, spacing: DFSpace.m) {
                    qrCode
                    addressDetails(address)
                }
            } else {
                DFEmptyState(symbol: "qrcode",
                             title: sharing.state?.enabled == true ? "等待可访问地址" : "开启后扫码传输",
                             message: sharing.state?.enabled == true
                                ? "服务成功监听局域网后，二维码和地址会显示在这里。"
                                : "开启开关后，二维码和访问地址会显示在这里。",
                             actionTitle: sharing.state?.enabled == true ? nil : "开启",
                             action: sharing.state?.enabled == true ? nil : { sharing.configure(enabled: true) })
                    .frame(maxWidth: .infinity)
            }
        }
    }

    @ViewBuilder private var qrCode: some View {
        if let qrImage = sharing.qrImage {
            Image(nsImage: qrImage)
                .interpolation(.none).resizable().scaledToFit()
                .frame(width: 144, height: 144).padding(DFSpace.s)
                .background(Color.white, in: RoundedRectangle(cornerRadius: DFRadius.card))
                .accessibilityLabel("用手机相机扫描二维码打开局域网传输网页")
        } else {
            DFEmptyState(symbol: "qrcode", title: "二维码不可用",
                         message: "仍可使用访问地址打开网页。")
                .frame(width: 160, height: 160)
        }
    }

    private func addressDetails(_ address: String) -> some View {
        VStack(alignment: .leading, spacing: DFSpace.s) {
            Text("用手机相机扫码")
                .font(DFType.bodyMedium)
            Text("发送照片或文字，也可取用 Mac 列表内容。")
                .font(DFType.detail)
                .dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
            if let state = sharing.state, state.addresses.count > 1 {
                Picker("访问地址", selection: Binding(
                    get: { liveAddress ?? "" },
                    set: { sharing.selectedAddress = $0 }
                )) {
                    ForEach(state.addresses, id: \.self) { Text($0).tag($0) }
                }
                .pickerStyle(.menu)
                .labelsHidden()
                .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Text(address)
                    .font(DFType.mono)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: DFSpace.s) {
                Button(addressCopied ? "已复制地址" : "复制地址", systemImage: "doc.on.doc") {
                    NSPasteboard.general.clearContents()
                    addressCopied = NSPasteboard.general.setString(address, forType: .string)
                    if !addressCopied { sharing.actionError = "未能写入系统剪贴板，请选择地址后手动复制。" }
                }
                .buttonStyle(.dfSecondary)
                Button("打开网页", systemImage: "arrow.up.right.square") {
                    guard let url = URL(string: address), NSWorkspace.shared.open(url) else {
                        sharing.actionError = "无法打开浏览器，请复制地址后手动访问。"
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
                if compact {
                    progress
                    actions
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if !compact { progress.frame(width: 260, alignment: .leading) }
            if !compact { actions.frame(width: 72) }
        }
        .padding(.vertical, DFSpace.m)
    }

    private var actions: some View {
        Group {
            if task.phase == .failed || task.phase == .cancelled {
                if task.canRetry { Button("重试") { store.send(.retryTransfer(id: task.id)) } }
            } else if task.phase != .completed {
                Button("取消") { store.send(.cancelTransfer(id: task.id)) }
            }
        }
        .buttonStyle(.dfPlain)
        .controlSize(.small)
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
