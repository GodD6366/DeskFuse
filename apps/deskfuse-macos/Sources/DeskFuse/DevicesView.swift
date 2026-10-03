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
    @State private var confirmLeave = false
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
        }
        .confirmationDialog("退出当前设备网络？", isPresented: $confirmLeave) {
            Button("退出网络", role: .destructive) { store.send(.leaveNetwork) }
        } message: {
            Text("这会解除本机与所有已配对设备的网络关系，不是断开某一台设备。")
        }
    }

    private var deviceListPane: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .top, spacing: 12) {
                    VStack(alignment: .leading, spacing: 5) {
                        Text(showsApplications ? "远程应用" : "设备")
                            .font(.system(size: 27, weight: .bold))
                        Text(showsApplications ? "选择设备，打开远端应用" : "发现并管理附近设备")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                    Button {
                        showManualPairing = true
                    } label: {
                        Image(systemName: "plus")
                            .font(.system(size: 15, weight: .medium))
                            .frame(width: 34, height: 34)
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.large)
                    .help("手动输入 IPv4 地址")
                    .accessibilityLabel("手动输入 IPv4 地址")
                    .popover(isPresented: $showManualPairing, arrowEdge: .top) {
                        manualPairPopover
                    }
                    Button { store.send(.refreshDevices) } label: {
                        Image(systemName: "arrow.clockwise")
                            .font(.system(size: 15, weight: .medium))
                            .frame(width: 34, height: 34)
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.large)
                    .help("刷新设备")
                    .accessibilityLabel("刷新设备")
                }

                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass")
                        .foregroundStyle(.secondary)
                    TextField("搜索设备名称、IP 地址…", text: $search)
                        .textFieldStyle(.plain)
                        .focused($searchFocused)
                        .onExitCommand { search = ""; searchFocused = false }
                    if !search.isEmpty {
                        Button { search = "" } label: {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundStyle(.secondary)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("清除设备搜索")
                    }
                    Button { } label: {
                        Image(systemName: "line.3.horizontal.decrease.circle")
                            .font(.system(size: 16, weight: .medium))
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .help("设备筛选")
                    .accessibilityLabel("设备筛选")
                }
                .font(.system(size: 13))
                .padding(.horizontal, 12)
                .frame(height: 38)
                .background(Color(nsColor: .quaternaryLabelColor).opacity(0.38), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .strokeBorder(searchFocused ? Color.accentColor.opacity(0.8) : .clear, lineWidth: 2)
                }
            }
            .padding(.horizontal, 24)
            .padding(.top, 28)
            .padding(.bottom, 14)

            if !snapshot.pairing.isEmpty {
                pairingNotice
                    .padding(.horizontal, 18)
                    .padding(.bottom, 8)
            }

            List(selection: $selected) {
                if localMatches {
                    Section {
                        localDeviceRow
                    } header: {
                        Text("本机")
                            .font(.system(size: 13, weight: .semibold))
                            .foregroundStyle(.secondary)
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
                        .foregroundStyle(.secondary)
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
        .background(Color(nsColor: .textBackgroundColor))
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
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(.secondary)
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
                                        Color.orange.opacity(0.2),
                                        Color.orange.opacity(0.1)
                                    ],
                                    startPoint: .topLeading,
                                    endPoint: .bottomTrailing
                                )
                            )
                            .frame(width: 40, height: 40)

                        Image(systemName: "person.crop.circle.badge.plus")
                            .font(.system(size: 18, weight: .medium))
                            .foregroundStyle(Color.orange)
                    }

                    VStack(alignment: .leading, spacing: 4) {
                        Text(request.name)
                            .font(.system(size: 14, weight: .semibold))
                        HStack(spacing: 4) {
                            Circle()
                                .fill(Color.orange)
                                .frame(width: 5, height: 5)
                            Text(request.incoming ? "请求加入设备网络" : "等待对端确认")
                                .font(.system(size: 12))
                                .foregroundStyle(.secondary)
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
                                Button {
                                    store.send(.reject(requestId: request.id), pairing: request.id)
                                } label: {
                                    Text("拒绝")
                                        .font(.system(size: 12, weight: .medium))
                                        .foregroundStyle(.secondary)
                                        .padding(.horizontal, 12)
                                        .padding(.vertical, 6)
                                }
                                .buttonStyle(.plain)
                                .background(Color(nsColor: .quaternaryLabelColor).opacity(0.25))
                                .clipShape(Capsule())

                                Button {
                                    store.send(.approve(requestId: request.id), pairing: request.id)
                                } label: {
                                    Text("允许")
                                        .font(.system(size: 12, weight: .medium))
                                        .foregroundStyle(.white)
                                        .padding(.horizontal, 14)
                                        .padding(.vertical, 6)
                                }
                                .buttonStyle(.plain)
                                .background(Color.orange)
                                .clipShape(Capsule())
                            }
                        }
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .background(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(Color.orange.opacity(0.06))
                )
                .overlay {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .strokeBorder(Color.orange.opacity(0.2), lineWidth: 1)
                }
            }
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Color.orange.opacity(0.04))
        )
        .overlay {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .strokeBorder(Color.orange.opacity(0.15), lineWidth: 1)
        }
    }

    private var manualPairPopover: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("手动输入 IPv4 地址")
                .font(.headline)
            Text("配对仍需对方确认。")
                .font(.callout)
                .foregroundStyle(.secondary)
            HStack(spacing: 8) {
                TextField("192.168.1.10", text: $ip)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(pairByIP)
                Button("配对", action: pairByIP)
                    .buttonStyle(.borderedProminent)
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
                    .font(.system(size: 14, weight: .semibold))
                    .lineLimit(1)
                HStack(spacing: 6) {
                    Circle()
                        .fill(device.connected ? Color.green : Color.secondary.opacity(0.5))
                        .frame(width: 7, height: 7)
                    Text(listDetail(for: device))
                        .font(.caption)
                        .foregroundStyle(.secondary)
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
                        .font(.system(size: 15, weight: .medium))
                        .frame(width: 26, height: 26)
                }
                .buttonStyle(.borderless)
                .disabled(!device.connected)
                .help(device.connected ? "打开远程应用" : "连接设备后可打开远程应用")
                .accessibilityLabel("打开 \(device.name) 的远程应用")
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .contentShape(Rectangle())
        .background(
            selected == device.id ? Color.accentColor.opacity(0.14) : .clear,
            in: RoundedRectangle(cornerRadius: 11, style: .continuous)
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
                    .font(.system(size: 14, weight: .semibold))
                    .lineLimit(1)
                Text(snapshot.runtimeStatus)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .contentShape(Rectangle())
        .background(
            selected == snapshot.localId ? Color.accentColor.opacity(0.14) : .clear,
            in: RoundedRectangle(cornerRadius: 11, style: .continuous)
        )
        .background(NativeListSelection())
        .tag(snapshot.localId)
        .listRowSeparator(.hidden)
        .listRowInsets(EdgeInsets(top: 2, leading: 8, bottom: 2, trailing: 8))
    }

    @ViewBuilder private var applicationPane: some View {
        if #available(macOS 14.2, *) {
            if let device = snapshot.devices.first(where: { $0.id == selected }) {
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
                        .foregroundStyle(.secondary)
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
            .padding(28)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Color(nsColor: .textBackgroundColor))
    }

    private func remoteDetail(_ device: Device) -> some View {
        VStack(alignment: .leading, spacing: 22) {
            deviceHeader(device)
            DeviceRoutesPanel(routes: device.routes) { store.send(.refreshDevices) }
            transferActivity
            operations(for: device)
        }
    }

    private var localDetail: some View {
        VStack(alignment: .leading, spacing: 22) {
            HStack(spacing: 16) {
            DeviceProductIconView(
                displayName: snapshot.localName,
                modelIdentifier: DeviceProductIconResolver.localModelIdentifier,
                size: 58
            )
                VStack(alignment: .leading, spacing: 6) {
                    Text(snapshot.localName).font(.system(size: 25, weight: .bold))
                    Label(snapshot.runtimeStatus, systemImage: "circle.fill")
                        .font(.callout)
                        .foregroundStyle(snapshot.error == nil ? Color.green : Color.orange)
                }
                Spacer(minLength: 0)
            }
            infoCard {
                infoRow("运行状态", symbol: "power", value: snapshot.runtimeStatus)
                Divider()
                infoRow("键鼠", symbol: "computermouse", value: snapshot.input.summary)
            }
            if snapshot.networkId != nil {
                VStack(alignment: .leading, spacing: 10) {
                    Button("退出设备网络", systemImage: "rectangle.portrait.and.arrow.right") { confirmLeave = true }
                        .buttonStyle(.bordered)
                        .tint(.red)
                    Text("退出后解除本机与所有已配对设备的网络关系，不是断开某一台设备。")
                        .font(.caption).foregroundStyle(.secondary)
                }
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
                    .font(.system(size: 25, weight: .bold))
                    .lineLimit(1)
                HStack(spacing: 8) {
                    Circle().fill(device.connected ? Color.green : Color.secondary.opacity(0.6)).frame(width: 10, height: 10)
                    Text(device.connected ? "在线" : "离线")
                        .font(.callout)
                        .foregroundStyle(device.connected ? Color.primary : Color.secondary)
                    Text("· \(device.endpoint)")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Text(device.detail)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            Menu {
                if #available(macOS 14.2, *) {
                    Button("打开远程应用", systemImage: "square.grid.2x2") {
                        RemoteApplicationManager.shared.open(device)
                    }
                    .disabled(!device.connected)
                }
                Divider()
                Button("刷新连接", systemImage: "arrow.clockwise") { store.send(.refreshDevices) }
            } label: {
                Image(systemName: "ellipsis")
                    .frame(width: 34, height: 34)
            }
            .menuStyle(.borderlessButton)
            .background(Color(nsColor: .quaternaryLabelColor).opacity(0.35), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .accessibilityLabel("设备操作")
        }
    }

    private var transferActivity: some View {
        deviceCard {
            HStack {
                Text("传输活动").font(.headline)
                Spacer()
                Button("查看全部") { store.selectedPage = .lanTransfer }
                    .buttonStyle(.link)
                    .font(.callout)
            }
            let recent = Array(snapshot.transfers.prefix(3))
            if recent.isEmpty {
                Text("暂无传输任务")
                    .font(.callout).foregroundStyle(.secondary)
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
                .font(.system(size: 22))
                .foregroundStyle(task.outgoing ? Color.accentColor : Color.green)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 5) {
                Text(task.name).font(.callout.weight(.medium)).lineLimit(1)
                Text(task.detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
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
                    .foregroundStyle(.secondary)
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
                .foregroundStyle(.secondary)
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
                    .buttonStyle(.bordered)
                } else if !device.trusted {
                    Button("邀请配对", systemImage: "person.badge.plus") {
                        store.send(.pair(peerId: device.id))
                    }
                    .buttonStyle(.borderedProminent)
                } else {
                    Button(device.connected ? "断开连接" : "重新连接", systemImage: device.connected ? "pin.slash" : "arrow.clockwise") {
                        store.send(device.connected ? .disconnect(peerId: device.id) : .connect(peerId: device.id))
                    }
                    .buttonStyle(.bordered)
                    Button("忘记设备", systemImage: "trash", role: .destructive) {
                        forgetTarget = device.id
                        confirmForget = true
                    }
                    .buttonStyle(.bordered)
                }
                Spacer(minLength: 0)
            }
            .controlSize(.large)
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
                .font(.system(size: 17))
                .frame(width: 24)
                .foregroundStyle(.secondary)
            Text(title)
                .frame(width: 62, alignment: .leading)
            Text(value.isEmpty ? "暂无信息" : value)
                .foregroundStyle(.secondary)
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

private struct DeviceDetailsSurface: ViewModifier {
    func body(content: Content) -> some View {
        content
            .background(Color(nsColor: .controlBackgroundColor).opacity(0.52), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .strokeBorder(Color(nsColor: .separatorColor).opacity(0.45))
            }
    }
}

struct PairingRow: View {
    @Bindable var store: AppStore
    let request: Pairing

    var body: some View {
        HStack(alignment: .center, spacing: 16) {
            // Icon container with gradient background
            ZStack {
                Circle()
                    .fill(
                        LinearGradient(
                            colors: [
                                Color.accentColor.opacity(0.18),
                                Color.accentColor.opacity(0.08)
                            ],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        )
                    )
                    .frame(width: 56, height: 56)

                Image(systemName: "person.crop.circle.badge.plus")
                    .font(.system(size: 24, weight: .medium))
                    .foregroundStyle(
                        LinearGradient(
                            colors: [Color.accentColor, Color.accentColor.opacity(0.8)],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        )
                    )
            }

            // Content
            VStack(alignment: .leading, spacing: 6) {
                Text(request.name)
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(.primary)

                HStack(spacing: 6) {
                    Circle()
                        .fill(Color.orange)
                        .frame(width: 6, height: 6)
                    Text(request.incoming ? "请求加入设备网络" : "等待对端确认")
                        .font(.system(size: 13))
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            // Actions
            if request.incoming {
                if store.pairingDecisions.contains(request.id) {
                    HStack(spacing: 8) {
                        ProgressView()
                            .controlSize(.small)
                            .scaleEffect(0.8)
                        Text("处理中")
                            .font(.system(size: 12, weight: .medium))
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                } else {
                    HStack(spacing: 10) {
                        Button {
                            store.send(.reject(requestId: request.id), pairing: request.id)
                        } label: {
                            Text("拒绝")
                                .font(.system(size: 13, weight: .semibold))
                                .foregroundStyle(.secondary)
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
                                .font(.system(size: 13, weight: .semibold))
                                .foregroundStyle(.white)
                                .padding(.horizontal, 20)
                                .padding(.vertical, 9)
                        }
                        .buttonStyle(.plain)
                        .background(
                            LinearGradient(
                                colors: [
                                    Color.accentColor,
                                    Color.accentColor.opacity(0.9)
                                ],
                                startPoint: .topLeading,
                                endPoint: .bottomTrailing
                            )
                        )
                        .clipShape(Capsule())
                        .contentShape(Capsule())
                        .shadow(color: Color.accentColor.opacity(0.3), radius: 8, y: 4)
                    }
                }
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 18)
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(Color(nsColor: .controlBackgroundColor).opacity(0.6))
        )
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
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
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .strokeBorder(
                    LinearGradient(
                        colors: [
                            Color.white.opacity(0.15),
                            Color.white.opacity(0.05)
                        ],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    ),
                    lineWidth: 1
                )
        }
        .shadow(color: Color.black.opacity(0.08), radius: 12, y: 4)
        .shadow(color: Color.black.opacity(0.04), radius: 2, y: 1)
    }
}
