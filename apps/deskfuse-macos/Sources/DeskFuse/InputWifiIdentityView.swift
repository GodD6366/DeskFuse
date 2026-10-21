import SwiftUI
import DeskFuseEngine

/// 键鼠共享页的 Wi-Fi 标识展示数据：区分本机读到的 SSID、场景判定实际采用的 SSID、
/// 该网络保存的首选主控，以及各对端上报的 SSID，便于确认两端是否处在同一场景。
struct InputWifiIdentity: Equatable {
    enum Local: Equatable {
        case observed(String)
        /// 缺少定位授权时 macOS 不返回 SSID。
        case needsPermission
        /// 已授权但没有关联 Wi-Fi，例如只走有线网络。
        case notOnWifi
    }

    struct Peer: Equatable, Identifiable {
        let id: String
        let name: String
        let ssid: String
        /// 与场景判定采用的 SSID 是否一致；没有可比较的本机标识时为 nil。
        let matches: Bool?
    }

    let local: Local
    let effective: String?
    let adoptedFromPeer: Bool
    let preferredController: String?
    let peers: [Peer]

    init(
        observed: String?,
        effective: String?,
        rules: [(ssid: String, controller: String)],
        peerSsids: [(deviceId: String, ssid: String)],
        names: [String: String],
        localId: String,
        wifiGranted: Bool
    ) {
        if let observed {
            local = .observed(observed)
        } else {
            local = wifiGranted ? .notOnWifi : .needsPermission
        }
        self.effective = effective
        adoptedFromPeer = effective != nil && effective != observed
        let displayName: (String) -> String = { id in
            id == localId ? "这台 Mac" : names[id] ?? String(id.prefix(8))
        }
        preferredController = effective
            .flatMap { ssid in rules.first { $0.ssid == ssid }?.controller }
            .map(displayName)
        let reference = effective ?? observed
        peers = peerSsids
            .map { peer in
                Peer(id: peer.deviceId, name: displayName(peer.deviceId), ssid: peer.ssid,
                     matches: reference.map { $0 == peer.ssid })
            }
            .sorted { ($0.name, $0.id) < ($1.name, $1.id) }
    }

    /// 状态胶囊上的简短文案：优先显示场景判定采用的 SSID。
    var summary: String {
        if let ssid = effective { return ssid }
        switch local {
        case .observed(let ssid): return ssid
        case .needsPermission: return "未读取 Wi-Fi"
        case .notOnWifi: return "未连接 Wi-Fi"
        }
    }

    /// 缺少定位授权或有对端上报不一致时提示用户点开查看。
    var needsAttention: Bool {
        local == .needsPermission || peers.contains { $0.matches == false }
    }

    init(input: InputState, devices: [Device], localId: String, wifiGranted: Bool) {
        self.init(
            observed: input.observedSsid,
            effective: input.ssid,
            rules: input.rules.map { (ssid: $0.ssid, controller: $0.controller) },
            peerSsids: input.peerSsids.map { (deviceId: $0.deviceId, ssid: $0.ssid) },
            names: Dictionary(devices.map { ($0.id, $0.name) }, uniquingKeysWith: { first, _ in first }),
            localId: localId,
            wifiGranted: wifiGranted
        )
    }
}

/// 键鼠共享页的 Wi-Fi 状态胶囊：默认只显示 SSID，点击后弹出完整标识详情。
struct InputWifiIdentityView: View {
    let identity: InputWifiIdentity
    /// 场景判定的结论文案；接收端页面已单独展示时传 nil。
    let message: String?
    let onAuthorize: () -> Void
    @State private var showingDetail = false

    var body: some View {
        Button { showingDetail = true } label: {
            HStack(spacing: DFSpace.xs + 2) {
                Image(systemName: identity.local == .notOnWifi && identity.effective == nil ? "wifi.slash" : "wifi")
                    .font(DFType.icon(.small, weight: .semibold))
                    .foregroundStyle(identity.needsAttention ? DFPalette.warning : Color.secondary)
                Text(identity.summary).font(DFType.detail.weight(.medium)).lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: 160, alignment: .leading)
                    .fixedSize(horizontal: true, vertical: false)
                if identity.needsAttention {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .font(DFType.icon(.small))
                        .foregroundStyle(DFPalette.warning)
                }
            }
            .padding(.horizontal, DFSpace.s + 2)
            .frame(height: 24)
            .background(Color.secondary.opacity(DFOpacity.soft), in: Capsule())
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .help("查看 Wi-Fi 标识详情")
        .accessibilityLabel("Wi-Fi 标识：\(identity.summary)")
        .popover(isPresented: $showingDetail, arrowEdge: .bottom) {
            detail
                .padding(DFSpace.l)
                .frame(width: 380, alignment: .leading)
                .dfFloating()
        }
    }

    private var detail: some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            Label("Wi-Fi 标识", systemImage: "wifi").font(DFType.section)
            Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: DFSpace.l, verticalSpacing: DFSpace.s) {
                GridRow {
                    label("本机读取")
                    localValue
                }
                GridRow {
                    label("场景判定")
                    if let effective = identity.effective {
                        HStack(spacing: DFSpace.s) {
                            ssidText(effective)
                            if identity.adoptedFromPeer {
                                Text("采用对端上报").font(DFType.caption).dfSecondaryText()
                            }
                        }
                    } else {
                        Text("未确定").font(DFType.detail).dfSecondaryText()
                    }
                }
                GridRow {
                    label("首选主控")
                    Text(identity.preferredController ?? "该网络未保存主控").font(DFType.detail)
                }
                GridRow {
                    label("对端上报")
                    peerList
                }
            }
            if let message, !message.isEmpty {
                Text(message).font(DFType.caption).dfSecondaryText()
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .contain)
    }

    private func label(_ text: String) -> some View {
        Text(text).font(DFType.detail).dfSecondaryText().gridColumnAlignment(.leading)
    }

    private func ssidText(_ ssid: String) -> some View {
        Text(ssid).font(DFType.mono).textSelection(.enabled)
    }

    @ViewBuilder private var localValue: some View {
        switch identity.local {
        case .observed(let ssid):
            ssidText(ssid)
        case .needsPermission:
            HStack(spacing: DFSpace.s) {
                Text("未读取：需要定位授权才能读取 Wi-Fi 名称")
                    .font(DFType.detail).foregroundStyle(DFPalette.warning)
                    .fixedSize(horizontal: false, vertical: true)
                Button("前往授权", action: onAuthorize).buttonStyle(.dfSecondary)
            }
        case .notOnWifi:
            Text("未连接 Wi-Fi；有线网络会采用对端上报的标识").font(DFType.detail).dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder private var peerList: some View {
        if identity.peers.isEmpty {
            Text("暂无；对端需与本机建立输入链路后才会上报").font(DFType.detail).dfSecondaryText()
                .fixedSize(horizontal: false, vertical: true)
        } else {
            VStack(alignment: .leading, spacing: DFSpace.xs) {
                ForEach(identity.peers) { peer in
                    HStack(spacing: DFSpace.s) {
                        Text(peer.name).font(DFType.detail)
                        ssidText(peer.ssid)
                        if let matches = peer.matches {
                            Label(matches ? "一致" : "不一致",
                                  systemImage: matches ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                                .font(DFType.caption)
                                .foregroundStyle(matches ? DFPalette.success : DFPalette.warning)
                        }
                    }
                }
            }
        }
    }
}
