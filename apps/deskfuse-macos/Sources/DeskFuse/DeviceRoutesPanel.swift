import SwiftUI
import DeskFuseEngine

/// Source and health are supplied by Rust, not guessed from private-IP prefixes.
struct DeviceRoutesPanel: View {
    let routes: [DeviceRoute]
    let refresh: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 7) {
                Text("连接与路由").font(.headline)
                Image(systemName: "info.circle")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .help("候选地址来自已发现或手动配置的连接链路")
                Text("\(routes.count)").font(.caption).monospacedDigit().foregroundStyle(.secondary)
                Spacer()
                Button("刷新", systemImage: "arrow.clockwise", action: refresh)
                    .buttonStyle(.link)
                    .controlSize(.small)
            }
            if routes.isEmpty {
                Text("暂未发现连接地址。请确认对方应用正在运行后刷新，或通过 IP 添加设备。")
                    .font(.callout).foregroundStyle(.secondary)
            } else {
                VStack(spacing: 0) {
                    HStack(spacing: 12) {
                        Text("").frame(width: 24)
                        Text("地址").frame(maxWidth: .infinity, alignment: .leading)
                        Text("发现来源").frame(width: 128, alignment: .leading)
                        Text("状态").frame(width: 94, alignment: .leading)
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    Divider()
                    ForEach(routes, id: \.id) { route in
                        HStack(spacing: 12) {
                            Image(systemName: route.active ? "circle.inset.filled" : "circle")
                                .font(.system(size: 18))
                                .foregroundStyle(route.active ? Color.accentColor : Color.secondary)
                                .frame(width: 24)
                            VStack(alignment: .leading, spacing: 4) {
                                Text(route.endpoint)
                                    .font(.system(.callout, design: .monospaced))
                                    .textSelection(.enabled)
                                    .lineLimit(1)
                                if route.probeError != nil {
                                    Text("探测失败")
                                        .font(.caption)
                                        .foregroundStyle(.orange)
                                        .lineLimit(2)
                                } else if let timestamp = route.lastProbeUnixMs {
                                    Text("最近探测 \(Date(timeIntervalSince1970: Double(timestamp) / 1_000), style: .time)")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(route.source)
                                if !route.path.isEmpty { Text(route.path) }
                            }
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .frame(width: 128, alignment: .leading)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(route.status)
                                    .foregroundStyle(route.active ? Color.green : Color.secondary)
                                if let rtt = route.rttMs { Text(rtt == 0 ? "<1 ms" : "\(rtt) ms").monospacedDigit() }
                            }
                            .font(.caption)
                            .frame(width: 94, alignment: .leading)
                        }
                        .padding(.horizontal, 14)
                        .padding(.vertical, 12)
                        if route.id != routes.last?.id { Divider().padding(.horizontal, 14) }
                    }
                }
                .background(Color(nsColor: .controlBackgroundColor).opacity(0.52), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .strokeBorder(Color(nsColor: .separatorColor).opacity(0.45))
                }
                if routes.count == 1 {
                    Text("目前仅发现这一条候选地址，不代表设备只有一个网络接口。")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            Text("同一地址只显示一次。自动模式按实测握手延迟选择最快可达线路；刷新会立即重测候选地址，不会中断当前连接。")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(16)
        .background(Color(nsColor: .controlBackgroundColor).opacity(0.28), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .strokeBorder(Color(nsColor: .separatorColor).opacity(0.35))
        }
    }
}
