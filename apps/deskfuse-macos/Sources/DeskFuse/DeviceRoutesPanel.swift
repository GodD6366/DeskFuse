import SwiftUI
import DeskFuseEngine

/// Source and health are supplied by Rust, not guessed from private-IP prefixes.
struct DeviceRoutesPanel: View {
    let routes: [DeviceRoute]
    let refresh: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 7) {
                Text("连接与路由").font(DFType.section)
                Image(systemName: "info.circle")
                    .font(DFType.body)
                    .foregroundStyle(.secondary)
                    .help("候选地址来自已发现或手动配置的连接链路")
                Text("\(routes.count)").font(DFType.caption).monospacedDigit().foregroundStyle(.secondary)
                Spacer()
                Button("刷新", systemImage: "arrow.clockwise", action: refresh)
                    .buttonStyle(.link)
                    .controlSize(.small)
            }
            if routes.isEmpty {
                Text("暂未发现连接地址。请确认对方应用正在运行后刷新，或通过 IP 添加设备。")
                    .font(DFType.detail).foregroundStyle(.secondary)
            } else {
                ViewThatFits(in: .horizontal) {
                    routeTable
                    compactRoutes
                }

                if routes.count == 1 {
                    Text("目前仅发现这一条候选地址，不代表设备只有一个网络接口。")
                        .font(DFType.caption).foregroundStyle(.secondary)
                }
            }
            Text("同一地址只显示一次。自动模式按实测握手延迟选择最快可达线路；刷新会立即重测候选地址，不会中断当前连接。")
                .font(DFType.caption).foregroundStyle(.secondary)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(16)
    }

    private var routeTable: some View {
        VStack(spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text("").frame(width: 24)
                Text("地址").frame(minWidth: 160, maxWidth: .infinity, alignment: .leading)
                Text("发现来源").frame(width: 128, alignment: .leading)
                Text("状态").frame(width: 94, alignment: .leading)
            }
            .font(DFType.caption)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            Divider()
            ForEach(routes, id: \.id) { route in
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Image(systemName: route.active ? "circle.inset.filled" : "circle")
                        .font(DFType.icon(.medium))
                        .foregroundStyle(route.active ? DFPalette.accent : Color.secondary)
                        .frame(width: 24)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(route.endpoint)
                            .font(.system(.callout, design: .monospaced))
                            .textSelection(.enabled)
                            .lineLimit(1)
                        if route.probeError != nil {
                            Text("探测失败")
                                .font(DFType.caption)
                                .foregroundStyle(DFPalette.warning)
                                .lineLimit(2)
                        } else if let timestamp = route.lastProbeUnixMs {
                            Text("最近探测 \(Date(timeIntervalSince1970: Double(timestamp) / 1_000), style: .time)")
                                .font(DFType.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .frame(minWidth: 160, maxWidth: .infinity, alignment: .leading)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(route.source)
                        if !route.path.isEmpty { Text(route.path) }
                    }
                    .font(DFType.caption)
                    .foregroundStyle(.secondary)
                    .frame(width: 128, alignment: .leading)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(route.status)
                            .foregroundStyle(route.active ? DFPalette.success : Color.secondary)
                        if let rtt = route.rttMs { Text(rtt == 0 ? "<1 ms" : "\(rtt) ms").monospacedDigit() }
                    }
                    .font(DFType.caption)
                    .frame(width: 94, alignment: .leading)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                if route.id != routes.last?.id { Divider().padding(.horizontal, 14) }
            }
        }
    }

    /// Keep complete endpoints readable when the inspector is narrow.
    private var compactRoutes: some View {
        VStack(spacing: 0) {
            ForEach(routes, id: \.id) { route in
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: route.active ? "circle.inset.filled" : "circle")
                        .font(DFType.icon(.medium))
                        .foregroundStyle(route.active ? DFPalette.accent : Color.secondary)
                        .frame(width: 24)
                    VStack(alignment: .leading, spacing: 6) {
                        Text(route.endpoint)
                            .font(DFType.mono)
                            .textSelection(.enabled)
                            .fixedSize(horizontal: false, vertical: true)
                        HStack(spacing: 8) {
                            Text(route.source).foregroundStyle(.secondary)
                            Text(route.status).foregroundStyle(route.active ? DFPalette.success : Color.secondary)
                            if let rtt = route.rttMs {
                                Text(rtt == 0 ? "<1 ms" : "\(rtt) ms").monospacedDigit()
                            }
                        }
                        if !route.path.isEmpty { Text(route.path).foregroundStyle(.secondary) }
                        if route.probeError != nil {
                            Text("探测失败").foregroundStyle(DFPalette.warning)
                        } else if let timestamp = route.lastProbeUnixMs {
                            Text("最近探测 \(Date(timeIntervalSince1970: Double(timestamp) / 1_000), style: .time)")
                                .foregroundStyle(.secondary)
                        }
                    }
                    .font(DFType.detail)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(.vertical, 12)
                if route.id != routes.last?.id { Divider() }
            }
        }
    }

}
