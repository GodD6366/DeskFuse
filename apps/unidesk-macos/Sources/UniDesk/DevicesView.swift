import SwiftUI
import AppKit
import UniDeskEngine

struct DevicesView: View {
    @Bindable var store: AppStore
    let snapshot: EngineSnapshot
    @State private var selected: String?
    @State private var search = ""
    @State private var ip = ""
    @State private var confirmForget = false
    @State private var forgetTarget: String?
    @State private var confirmLeave = false
    @FocusState private var searchFocused: Bool

    private var matches: [Device] {
        snapshot.devices.filter { search.isEmpty || $0.name.localizedCaseInsensitiveContains(search) || $0.endpoint.localizedCaseInsensitiveContains(search) || $0.routes.contains { $0.endpoint.localizedCaseInsensitiveContains(search) } }
    }

    var body: some View {
        HSplitView {
            VStack(spacing: 0) {
                VStack(alignment: .leading, spacing: 16) {
                    Text("设备").font(.system(size: 24, weight: .bold))
                    Text("发现并管理附近设备").foregroundStyle(.secondary)
                    HStack(spacing: 8) {
                        Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                        TextField("搜索设备", text: $search).textFieldStyle(.plain).focused($searchFocused)
                        if !search.isEmpty {
                            Button { search = "" } label: { Image(systemName: "xmark.circle.fill") }
                                .buttonStyle(.plain).accessibilityLabel("清除搜索")
                        }
                    }
                    .padding(10).background(Color(nsColor: .textBackgroundColor), in: Capsule())
                }.padding(20)
                List(selection: $selected) {
                    if search.isEmpty || snapshot.localName.localizedCaseInsensitiveContains(search) {
                        Section("本机") {
                            deviceRow(name: snapshot.localName, detail: snapshot.runtimeStatus, symbol: "laptopcomputer", connected: snapshot.error == nil)
                                .tag(snapshot.localId)
                        }
                    }
                    if matches.contains(where: { $0.trusted && !$0.blocked }) {
                        Section("已配对") { ForEach(matches.filter { $0.trusted && !$0.blocked }, id: \.id) { device in row(device) } }
                    }
                    if matches.contains(where: { !$0.trusted && !$0.blocked }) {
                        Section("附近设备") { ForEach(matches.filter { !$0.trusted && !$0.blocked }, id: \.id) { device in row(device) } }
                    }
                    if matches.contains(where: \.blocked) {
                        Section("已阻止") { ForEach(matches.filter(\.blocked), id: \.id) { device in row(device) } }
                    }
                    if matches.isEmpty {
                        Text(search.isEmpty ? "尚未发现其他设备" : "没有匹配的设备")
                            .font(.callout).foregroundStyle(.secondary).listRowSeparator(.hidden)
                    }
                }
                .listStyle(.inset).scrollContentBackground(.hidden)
                Divider().padding(.horizontal, 20)
                VStack(alignment: .leading, spacing: 10) {
                    Text("通过 IP 添加设备").font(.callout.weight(.semibold))
                    Text("输入对方的 IPv4 地址；配对仍需对方确认。")
                        .font(.caption).foregroundStyle(.secondary)
                    HStack {
                        TextField("192.168.1.10", text: $ip).textFieldStyle(.roundedBorder).onSubmit(pairByIP)
                        Button("配对", action: pairByIP).disabled(ip.trimmingCharacters(in: .whitespaces).isEmpty)
                    }
                }.padding(20)
            }
            .frame(minWidth: 240, idealWidth: 280, maxWidth: 320, maxHeight: .infinity)
            .background(Color(nsColor: .controlBackgroundColor))

            ScrollView {
                VStack(alignment: .leading, spacing: 28) {
                    if let message = snapshot.pairingMessage {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("配对操作").font(.caption.weight(.semibold))
                            Text(message).font(.callout).textSelection(.enabled)
                        }.foregroundStyle(.secondary)
                    }
                    ForEach(snapshot.pairing, id: \.id) { request in PairingRow(store: store, request: request) }
                    if selected == snapshot.localId {
                        Text(snapshot.localName).font(.system(size: 26, weight: .bold))
                        Label("这台 Mac", systemImage: "laptopcomputer").foregroundStyle(.secondary)
                        VStack(spacing: 0) {
                            infoRow("运行状态", symbol: "power", value: snapshot.runtimeStatus)
                            Divider()
                            infoRow("键鼠", symbol: "computermouse", value: snapshot.input.summary)
                        }.padding(20).modifier(DeviceDetailsSurface())
                    } else if let device = snapshot.devices.first(where: { $0.id == selected }) {
                        detail(device)
                    } else {
                        ContentUnavailableView("设备网络", systemImage: "desktopcomputer",
                                               description: Text("选择设备查看详情，或搜索附近设备。"))
                            .frame(maxWidth: .infinity)
                    }
                    Divider()
                    GroupBox("跨设备更新 · 这台 Mac") {
                        UpdateControls(store: store, update: snapshot.update)
                            .padding(8).frame(maxWidth: .infinity, alignment: .leading)
                    }
                    if snapshot.networkId != nil {
                        Divider()
                        Button("退出设备网络", role: .destructive) { confirmLeave = true }.buttonStyle(.plain)
                    }
                }.padding(28).frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(minWidth: 340, maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(nsColor: .textBackgroundColor))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .toolbar {
            Button("搜索设备", systemImage: "magnifyingglass") { searchFocused = true }.keyboardShortcut("f", modifiers: .command)
            Button("刷新设备", systemImage: "arrow.clockwise") { store.send(.refreshDevices) }
        }
        .onChange(of: snapshot.devices.map(\.id), initial: true) { _, ids in
            if selected == snapshot.localId { return }
            if let selected, ids.contains(selected) { return }
            selected = snapshot.devices.first(where: \.trusted)?.id ?? ids.first ?? snapshot.localId
        }
        .confirmationDialog("忘记此设备并撤销信任？", isPresented: $confirmForget) {
            Button("忘记设备", role: .destructive) { if let forgetTarget { store.send(.forget(peerId: forgetTarget)) } }
        }
        .confirmationDialog("退出当前设备网络？", isPresented: $confirmLeave) {
            Button("退出网络", role: .destructive) { store.send(.leaveNetwork) }
        }
    }

    private func row(_ device: Device) -> some View {
        deviceRow(name: device.name, detail: device.detail, symbol: "desktopcomputer", connected: device.connected).tag(device.id)
    }

    private func deviceRow(name: String, detail: String, symbol: String, connected: Bool) -> some View {
        HStack(spacing: 12) {
            Image(systemName: symbol).font(.system(size: 24, weight: .light)).frame(width: 30).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 6) {
                Text(name).font(.system(size: 13, weight: .semibold)).lineLimit(1)
                HStack(alignment: .top, spacing: 6) {
                    Circle().fill(connected ? Color.green : Color.secondary).frame(width: 6, height: 6).padding(.top, 4)
                    Text(detail).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                }
            }
        }.padding(.vertical, 10).listRowSeparator(.hidden)
    }

    private func detail(_ device: Device) -> some View {
        VStack(alignment: .leading, spacing: 28) {
            VStack(alignment: .leading, spacing: 12) {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 12) { Text(device.name).font(.system(size: 26, weight: .bold)); connectionBadge(device) }
                    VStack(alignment: .leading, spacing: 8) { Text(device.name).font(.system(size: 26, weight: .bold)); connectionBadge(device) }
                }
                Text(device.detail).foregroundStyle(.secondary)
            }
            VStack(spacing: 0) {
                infoRow("连接", symbol: "link", value: device.connected ? "已连接" : "未连接")
                Divider()
                infoRow("当前地址", symbol: "mappin.and.ellipse",
                        value: device.routes.first(where: \.active)?.endpoint ?? (device.connected ? "活动链路尚未报告" : "未连接"))
                Divider()
                infoRow("信任", symbol: "shield", value: device.blocked ? "已阻止" : device.trusted ? "已配对" : "尚未配对")
                Divider()
                infoRow("键鼠", symbol: "computermouse", value: device.inputReason)
            }.padding(.horizontal, 20).modifier(DeviceDetailsSurface())
            DeviceRoutesPanel(routes: device.routes) {
                store.send(.refreshDevices)
            }
            VStack(alignment: .leading, spacing: 12) {
                Text("优先路径").font(.headline)
                Picker("优先路径", selection: Binding(get: { device.routePreference }, set: { store.send(.setRoute(peerId: device.id, preference: $0)) })) {
                    Text("自动").tag(RoutePreference.automatic); Text("雷电").tag(RoutePreference.thunderbolt)
                    Text("有线").tag(RoutePreference.ethernet); Text("Wi-Fi").tag(RoutePreference.wifi)
                }.pickerStyle(.segmented).labelsHidden()
                Text("这是链路类型偏好，不会绑定单个 IP。自动会选当前测得最快的可达地址；点刷新会重新探测并在有更快或更符合偏好的线路时切换。").font(.caption).foregroundStyle(.secondary)
            }
            HStack(spacing: 18) {
                if device.blocked { Button("允许重新配对") { store.send(.allowPairing(peerId: device.id)) } }
                else if !device.trusted { Button("邀请配对") { store.send(.pair(peerId: device.id)) }.modifier(NativeGlassAction()) }
                else {
                    Button(device.connected ? "断开连接" : "重新连接") { store.send(device.connected ? .disconnect(peerId: device.id) : .connect(peerId: device.id)) }.modifier(NativeGlassAction())
                    Button("忘记设备", role: .destructive) { forgetTarget = device.id; confirmForget = true }.buttonStyle(.plain)
                }
            }.controlSize(.large)
        }
    }

    private func connectionBadge(_ device: Device) -> some View {
        Label(device.connected ? "已连接" : "未连接", systemImage: "circle.fill")
            .font(.caption).foregroundStyle(device.connected ? Color.green : Color.secondary)
            .padding(.horizontal, 10).padding(.vertical, 6)
            .background((device.connected ? Color.green : Color.secondary).opacity(0.1), in: Capsule()).fixedSize()
    }

    private func infoRow(_ title: String, symbol: String, value: String) -> some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: symbol).font(.system(size: 19)).frame(width: 24).foregroundStyle(.secondary)
            Text(title).frame(width: 48, alignment: .leading)
            Text(value.isEmpty ? "暂无信息" : value).foregroundStyle(.secondary).textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }.font(.callout).padding(.vertical, 18)
    }
    private func pairByIP() { guard !ip.trimmingCharacters(in: .whitespaces).isEmpty else { return }; store.send(.pairAt(ip: ip)) }
}

private struct DeviceDetailsSurface: ViewModifier {
    func body(content: Content) -> some View {
        content.background(Color(nsColor: .controlBackgroundColor).opacity(0.4), in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(.separator.opacity(0.5)))
    }
}
struct PairingRow: View {
    @Bindable var store: AppStore
    let request: Pairing
    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                Label(request.name, systemImage: "person.crop.circle.badge.questionmark").font(.headline)
                Text(request.incoming ? "请求加入设备网络" : "等待对端确认").foregroundStyle(.secondary)
                if request.incoming {
                    HStack {
                        Button("拒绝") { store.send(.reject(requestId: request.id), pairing: request.id) }
                        Button("允许配对") { store.send(.approve(requestId: request.id), pairing: request.id) }.buttonStyle(.borderedProminent)
                    }.disabled(store.pairingDecisions.contains(request.id))
                    if store.pairingDecisions.contains(request.id) { ProgressView("正在处理…").controlSize(.small) }
                }
            }.frame(maxWidth: .infinity, alignment: .leading).padding(8)
        }
    }
}
