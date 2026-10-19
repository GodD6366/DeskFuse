import SwiftUI
import AppKit
import DeskFuseEngine

/// Device management keeps the list and inspector visible at the same time.
/// The connection engine remains the source of truth; this file only changes
/// the presentation of its snapshot and routes existing commands unchanged.
struct DevicesView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    let searchRequest: Int
    let localSelectionRequest: Int
    var showsApplications = false
    @State private var selected: String?
    @State private var search = ""
    @State private var ip = ""
    @State private var showManualPairing = false
    @State private var confirmForget = false
    @State private var forgetTarget: String?
    @State private var confirmBlock = false
    @State private var blockTarget: String?
    @State private var confirmLeave = false
    @State private var confirmBatchUpdate = false
    @State private var batchUpdateIDs: [String] = []
    @FocusState private var searchFocused: Bool

    private var matches: [Device] {
        snapshot.devices
            .filter {
                search.isEmpty
                    || $0.name.localizedCaseInsensitiveContains(search)
                    || $0.endpoint.localizedCaseInsensitiveContains(search)
                    || $0.detail.localizedCaseInsensitiveContains(search)
                    || $0.routes.contains { $0.endpoint.localizedCaseInsensitiveContains(search) }
            }
            .sorted {
                if $0.connected != $1.connected { return $0.connected && !$1.connected }
                return $0.name.localizedStandardCompare($1.name) == .orderedAscending
            }
    }

    private var paired: [Device] { matches.filter { $0.trusted && !$0.blocked } }
    private var batchUpdateCandidates: [Device] {
        paired.filter { $0.connected && $0.canTriggerUpdate }
    }
    private var nearby: [Device] { matches.filter { !$0.trusted && !$0.blocked } }
    private var blocked: [Device] { matches.filter(\.blocked) }
    private var localMatches: Bool {
        search.isEmpty
            || snapshot.localName.localizedCaseInsensitiveContains(search)
            || "这台 Mac".localizedCaseInsensitiveContains(search)
    }

    var body: some View {
        HSplitView {
            deviceListPane
                .frame(minWidth: 300, idealWidth: 340, maxWidth: 380, maxHeight: .infinity)

            Group {
                if showsApplications {
                    applicationPane
                } else {
                    detailPane
                }
            }
            .frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onChange(of: searchRequest) { _, _ in searchFocused = true }
        .onChange(of: localSelectionRequest) { _, _ in selected = snapshot.localId }
        .onChange(of: snapshot.devices.map(\.id), initial: true) { _, ids in
            if selected == snapshot.localId || (selected != nil && ids.contains(selected!)) { return }
            selected = paired.first?.id ?? nearby.first?.id ?? blocked.first?.id ?? snapshot.localId
        }
        .confirmationDialog("忘记此设备并撤销信任？", isPresented: $confirmForget) {
            Button("忘记设备", role: .destructive) {
                if let forgetTarget { store.send(.forget(peerId: forgetTarget)) }
            }
        } message: {
            Text("这会断开连接并移除配对关系。设备仍可被发现和重新配对，不会被拉黑。")
        }
        .confirmationDialog("拉黑此设备？", isPresented: $confirmBlock) {
            Button("拉黑设备", role: .destructive) {
                if let blockTarget { store.send(.block(peerId: blockTarget)) }
            }
        } message: {
            Text("这会断开连接并阻止该设备配对，直到你允许重新配对。")
        }
        .confirmationDialog("退出当前设备网络？", isPresented: $confirmLeave) {
            Button("退出网络", role: .destructive) { store.send(.leaveNetwork) }
        } message: {
            Text("这会解除本机与所有已配对设备的网络关系，不是断开某一台设备。")
        }
        .confirmationDialog("批量触发设备更新？", isPresented: $confirmBatchUpdate) {
            Button("触发 \(batchUpdateIDs.count) 台设备") {
                store.send(.triggerPeerUpdates(peerIds: batchUpdateIDs))
                batchUpdateIDs = []
            }
        } message: {
            Text("将请求当前在线且支持主动更新的配对设备开始各自的更新流程。每台设备仍按本地版本和安全条件执行。")
        }
    }

    private var deviceListPane: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .top, spacing: 12) {
                    VStack(alignment: .leading, spacing: 5) {
                        HStack(spacing: 8) {
                            Text("设备列表")
                                .font(DFType.title)
                        }
                        Text(showsApplications ? "选择设备，打开远端应用" : "发现并管理附近设备")
                            .font(.callout)
                            .dfSecondaryText()
                    }
                    Spacer(minLength: 0)
                    Button {
                        showManualPairing = true
                    } label: {
                        Image(systemName: "plus")
                            .font(DFType.title.weight(.medium))
                            .frame(width: DFMetrics.controlHeight, height: DFMetrics.controlHeight)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.borderless)
                    .modifier(NativeGlassBackdrop(cornerRadius: DFRadius.control))
                    .controlSize(.small)
                    .help("手动输入 IPv4 地址")
                    .accessibilityLabel("手动输入 IPv4 地址")
                    .popover(isPresented: $showManualPairing, arrowEdge: .top) {
                        manualPairPopover.dfFloating()
                    }
                    Button { store.send(.refreshDevices) } label: {
                        Image(systemName: "arrow.clockwise")
                            .font(DFType.title.weight(.medium))
                            .frame(width: DFMetrics.controlHeight, height: DFMetrics.controlHeight)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.borderless)
                    .modifier(NativeGlassBackdrop(cornerRadius: DFRadius.control))
                    .controlSize(.small)
                    .help("刷新设备")
                    .accessibilityLabel("刷新设备")
                    if !showsApplications {
                        Button {
                            let ids = batchUpdateCandidates.map(\.id)
                            guard !ids.isEmpty else { return }
                            batchUpdateIDs = ids
                            confirmBatchUpdate = true
                        } label: {
                            Image(systemName: "arrow.down.app")
                                .font(DFType.title.weight(.medium))
                                .frame(width: DFMetrics.controlHeight, height: DFMetrics.controlHeight)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.borderless)
                        .modifier(NativeGlassBackdrop(cornerRadius: DFRadius.control))
                        .controlSize(.small)
                        .disabled(batchUpdateCandidates.isEmpty)
                        .help("批量触发 \(batchUpdateCandidates.count) 台设备更新")
                        .accessibilityLabel("批量触发设备更新")
                    }
                }

                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass")
                        .dfSecondaryText()
                    TextField("搜索设备名称、IP 地址…", text: $search)
                        .textFieldStyle(.plain)
                        .focused($searchFocused)
                        .onExitCommand { search = ""; searchFocused = false }
                    if !search.isEmpty {
                        Button { search = "" } label: {
                            Image(systemName: "xmark.circle.fill")
                                .dfSecondaryText()
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("清除设备搜索")
                    }
                }
                .font(DFType.body)
                .padding(.horizontal, 12)
                .frame(height: DFMetrics.controlHeight)
                .modifier(NativeGlassBackdrop(cornerRadius: DFRadius.control))
                .overlay {
                    RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
                        .strokeBorder(searchFocused ? DFPalette.accent.opacity(0.8) : .clear, lineWidth: 2)
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 18)
            .padding(.bottom, 14)

            if !snapshot.pairing.isEmpty {
                pairingNotice
                    .padding(.horizontal, 18)
                    .padding(.bottom, 8)
            }

            List(selection: $selected) {
                if localMatches && !showsApplications {
                    Section {
                        localDeviceRow
                    } header: {
                        Text("本机")
                            .font(DFType.section)
                            .dfSecondaryText()
                            .textCase(nil)
                            .padding(.top, 8)
                            .padding(.bottom, 4)
                    }
                }
                deviceSection("已配对", count: paired.count, devices: paired)
                deviceSection("附近", count: nearby.count, devices: nearby)
                deviceSection("已阻止", count: blocked.count, devices: blocked)
                if paired.isEmpty && nearby.isEmpty && blocked.isEmpty {
                    Text(search.isEmpty ? "尚未发现其他设备" : "没有匹配的设备")
                        .font(.callout)
                        .dfSecondaryText()
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .listRowSeparator(.hidden)
                        .padding(.vertical, 20)
                }
            }
            .listStyle(.inset)
            // List's primary action handles double-click without competing
            // with native single-click selection or the row's launch button.
            .contextMenu(forSelectionType: String.self) { _ in
                EmptyView()
            } primaryAction: { ids in
                guard ids.count == 1, let id = ids.first,
                      let device = snapshot.devices.first(where: { $0.id == id }),
                      device.trusted, device.connected, !device.blocked else { return }
                selected = id
                if !showsApplications, #available(macOS 14.2, *) {
                    RemoteApplicationManager.shared.open(device)
                }
            }
            .scrollContentBackground(.hidden)
            .environment(\.defaultMinListRowHeight, 52)
        }
        .background(.clear)
    }

    @ViewBuilder
    private func deviceSection(_ title: String, count: Int, devices: [Device]) -> some View {
        if !devices.isEmpty {
            Section {
                ForEach(devices, id: \.id) { device in
                    deviceRow(device)
                }
            } header: {
                Text("\(title)（\(count)）")
                    .font(DFType.section)
                    .dfSecondaryText()
                    .textCase(nil)
                    .padding(.top, 8)
                    .padding(.bottom, 4)
            }
        }
    }

    private var pairingNotice: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(snapshot.pairing, id: \.id) { request in
                HStack(spacing: 12) {
                    // Animated icon container
                    ZStack {
                        Circle()
                            .fill(
                                LinearGradient(
                                    colors: [
                                        DFPalette.warning.opacity(0.2),
                                        DFPalette.warning.opacity(0.1)
                                    ],
                                    startPoint: .topLeading,
                                    endPoint: .bottomTrailing
                                )
                            )
                            .frame(width: 40, height: 40)

                        Image(systemName: "person.crop.circle.badge.plus")
                            .font(DFType.icon(.medium).weight(.medium))
                            .foregroundStyle(DFPalette.warning)
                    }

                    VStack(alignment: .leading, spacing: 4) {
                        Text(request.name)
                            .font(DFType.icon(.regular).weight(.semibold))
                        HStack(spacing: 4) {
                            Circle()
                                .fill(DFPalette.warning)
                                .frame(width: 5, height: 5)
                            Text(request.incoming ? "请求加入设备网络" : "等待对端确认")
                                .font(DFType.detail)
                                .dfSecondaryText()
                        }
                    }

                    Spacer(minLength: 4)

                    if request.incoming {
                        if store.pairingDecisions.contains(request.id) {
                            ProgressView()
                                .controlSize(.small)
                                .scaleEffect(0.75)
                        } else {
                            HStack(spacing: 6) {
                                Button("拒绝") {
                                    store.send(.reject(requestId: request.id), pairing: request.id)
                                }
                                .buttonStyle(DFButtonStyle(compact: true))
                                Button("允许") {
                                    store.send(.approve(requestId: request.id), pairing: request.id)
                                }
                                .buttonStyle(DFButtonStyle(kind: .primary, compact: true))
                            }
                        }
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .background(
                    RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous)
                        .fill(DFPalette.warning.opacity(0.06))
                )
                .overlay {
                    RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous)
                        .strokeBorder(DFPalette.warning.opacity(0.2), lineWidth: 1)
                }
            }
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous)
                .fill(DFPalette.warning.opacity(0.04))
        )
        .overlay {
            RoundedRectangle(cornerRadius: DFRadius.card, style: .continuous)
                .strokeBorder(DFPalette.warning.opacity(0.15), lineWidth: 1)
        }
    }

    private var manualPairPopover: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("手动输入 IPv4 地址")
                .font(.headline)
            Text("配对仍需对方确认。")
                .font(.callout)
                .dfSecondaryText()
            HStack(spacing: 8) {
                TextField("192.168.1.10", text: $ip)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(pairByIP)
                Button("配对", action: pairByIP)
                    .buttonStyle(.dfPrimary)
                    .disabled(ip.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(16)
        .frame(width: 300)
    }

    private func deviceRow(_ device: Device) -> some View {
        HStack(spacing: 12) {
            DeviceProductIconView(
                displayName: device.name,
                size: 42
            )

            VStack(alignment: .leading, spacing: 5) {
                Text(device.name)
                    .font(DFType.icon(.regular).weight(.semibold))
                    .lineLimit(1)
                HStack(spacing: 6) {
                    Circle()
                        .fill(device.connected ? Color.green : Color.secondary.opacity(0.5))
                        .frame(width: 7, height: 7)
                    Text(listDetail(for: device))
                        .font(.caption)
                        .dfSecondaryText()
                        .lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            if device.trusted && !device.blocked, #available(macOS 14.2, *) {
                Button {
                    selected = device.id
                    if !showsApplications { RemoteApplicationManager.shared.open(device) }
                } label: {
                    Image(systemName: "square.grid.2x2")
                        .font(DFType.title.weight(.medium))
                        .frame(width: 26, height: 26)
                }
                .buttonStyle(.borderless)
                .disabled(!device.connected || !snapshot.input.canOperateRemoteApplications)
                .help(!snapshot.input.canOperateRemoteApplications ? RemoteApplicationManager.viewerRestriction
                      : device.connected ? "打开远程应用" : "连接设备后可打开远程应用")
                .accessibilityLabel("打开 \(device.name) 的远程应用")
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .contentShape(Rectangle())
        .background(
            selected == device.id ? DFPalette.selection : .clear,
            in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
        )
        .background(NativeListSelection())
        .tag(device.id)
        .listRowSeparator(.hidden)
        .listRowInsets(EdgeInsets(top: 2, leading: 8, bottom: 2, trailing: 8))
    }

    private var localDeviceRow: some View {
        HStack(spacing: 12) {
            DeviceProductIconView(
                displayName: snapshot.localName,
                modelIdentifier: DeviceProductIconResolver.localModelIdentifier,
                size: 42
            )
            VStack(alignment: .leading, spacing: 5) {
                Text(snapshot.localName)
                    .font(DFType.icon(.regular).weight(.semibold))
                    .lineLimit(1)
                Text(snapshot.runtimeStatus)
                    .font(.caption)
                    .dfSecondaryText()
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .contentShape(Rectangle())
        .background(
            selected == snapshot.localId ? DFPalette.selection : .clear,
            in: RoundedRectangle(cornerRadius: DFRadius.control, style: .continuous)
        )
        .background(NativeListSelection())
        .tag(snapshot.localId)
        .listRowSeparator(.hidden)
        .listRowInsets(EdgeInsets(top: 2, leading: 8, bottom: 2, trailing: 8))
    }

    @ViewBuilder private var applicationPane: some View {
        if #available(macOS 14.2, *) {
            if !RemoteApplicationManager.shared.enabled {
                FeatureGateView(
                    symbol: "square.grid.2x2",
                    symbolColor: .teal,
                    title: "远程应用",
                    message: "在这台 Mac 上打开另一台电脑的应用窗口。",
                    bullets: [
                        "应用窗口像本地窗口一样显示与操作",
                        "可同时打开多台设备的应用",
                        "支持 macOS 与 Windows 远端",
                    ],
                    permission: .screenRecording,
                    actionTitle: "启用远程应用",
                    action: { RemoteApplicationManager.shared.enabled = true },
                    secondaryTitle: "打开设置",
                    secondaryAction: { store.openSettings(tab: SettingsCategory.general.rawValue) }
                )
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if !snapshot.input.canOperateRemoteApplications {
                ContentUnavailableView("当前无法操作远程应用", systemImage: "keyboard",
                    description: Text(RemoteApplicationManager.viewerRestriction))
            } else if let device = snapshot.devices.first(where: { $0.id == selected }) {
                if device.blocked {
                    ContentUnavailableView("设备已阻止", systemImage: "hand.raised",
                        description: Text("在设备页解除阻止并配对后，即可查看应用列表。"))
                } else if !device.trusted {
                    ContentUnavailableView("请先配对设备", systemImage: "lock",
                        description: Text("在设备页与 \(device.name) 完成配对后，即可打开远程应用。"))
                } else if !device.connected {
                    ContentUnavailableView("设备未连接", systemImage: "network.slash",
                        description: Text("等待 \(device.name) 连接后，将自动加载应用列表。"))
                } else {
                    RemoteApplicationCatalog(device: device).id(device.id)
                }
            } else {
                ContentUnavailableView("选择远端设备", systemImage: "square.grid.2x2",
                    description: Text("从左侧选择已连接的配对设备，在这里浏览并打开它的应用。"))
            }
        } else {
            ContentUnavailableView("需要更新 macOS", systemImage: "square.grid.2x2",
                description: Text("远程应用需要 macOS 14.2 或更新版本。"))
        }
    }

    private var detailPane: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                if let message = snapshot.pairingMessage {
                    Text(message)
                        .font(.caption)
                        .dfSecondaryText()
                        .textSelection(.enabled)
                }
                if selected == snapshot.localId {
                    localDetail
                } else if let device = snapshot.devices.first(where: { $0.id == selected }) {
                    remoteDetail(device)
                } else {
                    ContentUnavailableView("选择设备", systemImage: "laptopcomputer.and.iphone", description: Text("从左侧列表选择设备查看连接与传输状态。"))
                        .frame(maxWidth: .infinity, minHeight: 360)
                }
            }
            .padding(.horizontal, DFMetrics.pageInset)
            .padding(.vertical, DFMetrics.pageTopInset)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(.clear)
    }

    private func remoteDetail(_ device: Device) -> some View {
        VStack(alignment: .leading, spacing: 14) {
            deviceHeader(device)
                .padding(20)
                .dfCard()
            DeviceRoutesPanel(routes: device.routes) { store.send(.refreshDevices) }
                .dfCard()
            transferActivity
            operations(for: device)
        }
    }

    private var localDetail: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 16) {
            DeviceProductIconView(
                displayName: snapshot.localName,
                modelIdentifier: DeviceProductIconResolver.localModelIdentifier,
                size: 58
            )
                VStack(alignment: .leading, spacing: 6) {
                    Text(snapshot.localName).font(DFType.title)
                    Label(snapshot.runtimeStatus, systemImage: "circle.fill")
                        .font(.callout)
                        .foregroundStyle(snapshot.error == nil ? Color.green : DFPalette.warning)
                }
                Spacer(minLength: 0)
            }
            .padding(20)
            .dfCard()
            infoCard {
                infoRow("运行状态", symbol: "power", value: snapshot.runtimeStatus)
                Divider()
                infoRow("键鼠", symbol: "computermouse", value: snapshot.input.summary)
            }
            if snapshot.networkId != nil {
                VStack(alignment: .leading, spacing: 10) {
                    Button("退出设备网络", systemImage: "rectangle.portrait.and.arrow.right") { confirmLeave = true }
                        .buttonStyle(.dfDestructive)
                        .tint(.red)
                    Text("退出后解除本机与所有已配对设备的网络关系，不是断开某一台设备。")
                        .font(.caption).dfSecondaryText()
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(18)
                .dfCard()
            }
        }
    }

    private func deviceHeader(_ device: Device) -> some View {
        HStack(alignment: .top, spacing: 16) {
            DeviceProductIconView(
                displayName: device.name,
                size: 58
            )
            VStack(alignment: .leading, spacing: 7) {
                Text(device.name)
                    .font(DFType.title)
                    .lineLimit(1)
                HStack(spacing: 8) {
                    Circle().fill(device.connected ? Color.green : Color.secondary.opacity(0.6)).frame(width: 10, height: 10)
                    Text(device.connected ? "在线" : "离线")
                        .font(.callout)
                        .foregroundStyle(device.connected ? Color.primary : Color.secondary)
                    Text("· \(device.endpoint)")
                        .font(.callout)
                        .dfSecondaryText()
                        .lineLimit(1)
                }
                Text(device.detail)
                    .font(.callout)
                    .dfSecondaryText()
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            Menu {
                if #available(macOS 14.2, *) {
                    Button("打开远程应用", systemImage: "square.grid.2x2") {
                        RemoteApplicationManager.shared.open(device)
                    }
                    .disabled(!device.connected || !snapshot.input.canOperateRemoteApplications)
                    .help(RemoteApplicationManager.viewerRestriction)
                }
                Divider()
                Button("刷新连接", systemImage: "arrow.clockwise") { store.send(.refreshDevices) }
            } label: {
                Image(systemName: "ellipsis")
                    .frame(width: DFMetrics.controlHeight, height: DFMetrics.controlHeight)
            }
            .menuStyle(.borderlessButton)
            .modifier(NativeGlassBackdrop(cornerRadius: DFRadius.control))
            .accessibilityLabel("设备操作")
        }
    }

    private var transferActivity: some View {
        deviceCard {
            HStack {
                Text("传输活动").font(.headline)
                Spacer()
                Button("查看全部") { store.selectedPage = .files }
                    .buttonStyle(.link)
                    .font(.callout)
            }
            let recent = Array(snapshot.transfers.prefix(3))
            if recent.isEmpty {
                Text("暂无传输任务")
                    .font(.callout).dfSecondaryText()
                    .padding(.vertical, 8)
            } else {
                VStack(spacing: 0) {
                    ForEach(recent, id: \.id) { task in
                        transferRow(task)
                        if task.id != recent.last?.id { Divider().padding(.leading, 38) }
                    }
                }
            }
        }
    }

    private func transferRow(_ task: Transfer) -> some View {
        HStack(spacing: 10) {
            Image(systemName: task.outgoing ? "arrow.up.circle.fill" : "arrow.down.circle.fill")
                .font(DFType.pageTitle)
                .foregroundStyle(task.outgoing ? DFPalette.accent : Color.green)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 5) {
                Text(task.name).font(.callout.weight(.medium)).lineLimit(1)
                Text(task.detail).font(.caption).dfSecondaryText().lineLimit(1)
            }
            .frame(minWidth: 90, maxWidth: 180, alignment: .leading)
            if task.total > 0 {
                ProgressView(value: Double(task.transferred), total: Double(task.total))
                    .progressViewStyle(.linear)
                    .frame(maxWidth: 110)
            }
            Spacer(minLength: 4)
            Text(task.phaseTitle)
                .font(.caption)
                .foregroundStyle(task.phase == .completed ? Color.green : Color.secondary)
                .lineLimit(1)
            if task.phase == .completed {
                Image(systemName: "magnifyingglass")
                    .dfSecondaryText()
            } else if task.canRetry {
                Button { store.send(.retryTransfer(id: task.id)) } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .buttonStyle(.plain)
                .accessibilityLabel("重试传输")
            } else {
                Button { store.send(.cancelTransfer(id: task.id)) } label: {
                    Image(systemName: "xmark")
                }
                .buttonStyle(.plain)
                .dfSecondaryText()
                .accessibilityLabel("取消传输")
            }
        }
        .padding(.vertical, 11)
    }

    private func operations(for device: Device) -> some View {
        deviceCard {
            Text("操作").font(.headline)
            HStack(spacing: 12) {
                if device.blocked {
                    Button("允许重新配对", systemImage: "checkmark.shield") {
                        store.send(.allowPairing(peerId: device.id))
                    }
                    .buttonStyle(.dfSecondary)
                } else if !device.trusted {
                    Button("邀请配对", systemImage: "person.badge.plus") {
                        store.send(.pair(peerId: device.id))
                    }
                    .buttonStyle(.dfPrimary)
                } else {
                    Button(device.connected ? "断开连接" : "重新连接", systemImage: device.connected ? "pin.slash" : "arrow.clockwise") {
                        store.send(device.connected ? .disconnect(peerId: device.id) : .connect(peerId: device.id))
                    }
                    .buttonStyle(.dfSecondary)
                    Button("忘记设备", systemImage: "trash", role: .destructive) {
                        forgetTarget = device.id
                        confirmForget = true
                    }
                    .buttonStyle(.dfSecondary)
                    Button("触发更新", systemImage: "arrow.down.app") {
                        store.send(.triggerPeerUpdate(peerId: device.id))
                    }
                    .buttonStyle(.dfSecondary)
                    .disabled(!device.connected || !device.canTriggerUpdate)
                    .help(device.canTriggerUpdate
                          ? "请求 \(device.name) 立即检查并安装可用更新"
                          : "对端尚未声明支持主动更新")
                }
                if !device.blocked {
                    Button("拉黑设备", systemImage: "hand.raised", role: .destructive) {
                        blockTarget = device.id
                        confirmBlock = true
                    }
                    .buttonStyle(.dfSecondary)
                    .tint(.red)
                }
                Spacer(minLength: 0)
            }
            .controlSize(.small)
        }
    }

    private func infoCard(@ViewBuilder content: () -> some View) -> some View {
        VStack(spacing: 0, content: content)
            .padding(.horizontal, 18)
            .modifier(DeviceDetailsSurface())
    }

    private func deviceCard<Content: View>(@ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 12, content: content)
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .modifier(DeviceDetailsSurface())
    }

    private func infoRow(_ title: String, symbol: String, value: String) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: symbol)
                .font(DFType.title)
                .frame(width: 24)
                .dfSecondaryText()
            Text(title)
                .frame(width: 62, alignment: .leading)
            Text(value.isEmpty ? "暂无信息" : value)
                .dfSecondaryText()
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .font(.callout)
        .padding(.vertical, 15)
    }

    private func listDetail(for device: Device) -> String {
        let status = device.connected ? "在线" : (device.blocked ? "已阻止" : "离线")
        let endpoint = device.endpoint.isEmpty ? device.detail : device.endpoint
        return endpoint.isEmpty ? status : "\(status) · \(endpoint)"
    }

    private func symbol(for device: Device) -> String {
        let name = device.name.lowercased()
        if name.contains("iphone") || name.contains("ipad") || name.contains("手机") { return "iphone" }
        if name.contains("windows") || name.contains("desktop") { return "desktopcomputer" }
        return "laptopcomputer"
    }

    private func pairByIP() {
        let value = ip.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        store.send(.pairAt(ip: value))
        ip = ""
        showManualPairing = false
    }
}

/// 详情区的分组表面：统一走系统设置风格的浅灰圆角 block。
private struct DeviceDetailsSurface: ViewModifier {
    func body(content: Content) -> some View {
        content.dfCard()
    }
}

struct PairingRow: View {
    @Bindable var store: AppStore
    let request: Pairing

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .center, spacing: 16) {
                // Icon container with gradient background
                ZStack {
                    Circle()
                        .fill(DFPalette.brandGradient.opacity(0.15))
                        .frame(width: 56, height: 56)

                    Image(systemName: "person.crop.circle.badge.plus")
                        .font(DFType.icon(.large).weight(.medium))
                        .foregroundStyle(DFPalette.brandGradient)
                }

                // Content
                VStack(alignment: .leading, spacing: 6) {
                    Text(request.name)
                        .lineLimit(2)
                        .truncationMode(.tail)
                        .font(DFType.title.weight(.semibold))
                        .foregroundStyle(.primary)

                    HStack(spacing: 6) {
                        Circle()
                            .fill(DFPalette.warning)
                            .frame(width: 6, height: 6)
                        Text(request.incoming ? "请求加入设备网络" : "等待对端确认")
                            .font(DFType.body)
                            .dfSecondaryText()
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            if request.incoming {
                HStack {
                    Spacer(minLength: 0)
                    pairingActions
                        .lineLimit(1)
                        .fixedSize(horizontal: true, vertical: false)
                }
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 18)
        .background(
            RoundedRectangle(cornerRadius: DFRadius.panel, style: .continuous)
                .fill(Color(nsColor: .controlBackgroundColor).opacity(0.6))
        )
        .background(
            RoundedRectangle(cornerRadius: DFRadius.panel, style: .continuous)
                .fill(
                    LinearGradient(
                        colors: [
                            Color.white.opacity(0.08),
                            Color.white.opacity(0.02)
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )
        )
        .overlay {
            RoundedRectangle(cornerRadius: DFRadius.panel, style: .continuous)
                .strokeBorder(DFPalette.brandGradient.opacity(0.4), lineWidth: 1)
        }
        .shadow(color: Color.black.opacity(0.08), radius: 12, y: 4)
        .shadow(color: Color.black.opacity(0.04), radius: 2, y: 1)
    }

    @ViewBuilder
    private var pairingActions: some View {
        if store.pairingDecisions.contains(request.id) {
            HStack(spacing: 8) {
                ProgressView()
                    .controlSize(.small)
                    .scaleEffect(0.8)
                Text("处理中")
                    .font(DFType.detail.weight(.medium))
                    .dfSecondaryText()
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        } else {
            HStack(spacing: 10) {
                Button {
                    store.send(.reject(requestId: request.id), pairing: request.id)
                } label: {
                    Text("拒绝")
                        .font(DFType.section)
                        .dfSecondaryText()
                        .padding(.horizontal, 18)
                        .padding(.vertical, 9)
                }
                .buttonStyle(.plain)
                .background(Color(nsColor: .quaternaryLabelColor).opacity(0.3))
                .clipShape(Capsule())
                .contentShape(Capsule())
                .overlay {
                    Capsule()
                        .strokeBorder(Color(nsColor: .separatorColor).opacity(0.3), lineWidth: 0.5)
                }

                Button {
                    store.send(.approve(requestId: request.id), pairing: request.id)
                } label: {
                    Text("允许配对")
                        .font(DFType.section)
                        .foregroundStyle(.white)
                        .padding(.horizontal, 20)
                        .padding(.vertical, 9)
                }
                .buttonStyle(.plain)
                .background(DFPalette.brandGradient)
                /// 白字压在青色一端的对比度不足，压一层极淡黑色保证深浅色下可读。
                .overlay { Capsule().fill(Color.black.opacity(0.12)) }
                .clipShape(Capsule())
                .contentShape(Capsule())
                .shadow(color: DFPalette.accent.opacity(0.3), radius: 8, y: 4)
            }
        }
    }
}
