import SwiftUI
import UniDeskEngine

/// Source and health are supplied by Rust, not guessed from private-IP prefixes.
struct DeviceRoutesPanel: View {
    let routes: [DeviceRoute]
    let refresh: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("连接链路").font(.headline)
                Text("\(routes.count)").font(.caption).monospacedDigit().foregroundStyle(.secondary)
                Spacer()
                Button("刷新", systemImage: "arrow.clockwise", action: refresh).controlSize(.small)
            }
            if routes.isEmpty {
                Text("暂未发现连接地址。请确认对方应用正在运行后刷新，或通过 IP 添加设备。")
                    .font(.callout).foregroundStyle(.secondary)
            } else {
                VStack(spacing: 0) {
                    ForEach(routes, id: \.id) { route in
                        VStack(alignment: .leading, spacing: 8) {
                            HStack(alignment: .top, spacing: 8) {
                                Image(systemName: route.active ? "link" : "network")
                                    .foregroundStyle(route.active ? Color.green : Color.secondary)
                                Text(route.endpoint).font(.system(.callout, design: .monospaced))
                                    .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                                Spacer(minLength: 0)
                            }
                            HStack(alignment: .top, spacing: 8) {
                                Text(route.source)
                                if !route.path.isEmpty { Text(route.path) }
                                if let rtt = route.rttMs { Text(rtt == 0 ? "<1 ms" : "\(rtt) ms").monospacedDigit() }
                                Text(route.status).foregroundStyle(route.active ? Color.green : Color.secondary)
                            }.font(.caption).foregroundStyle(.secondary)
                            if let timestamp = route.lastProbeUnixMs {
                                HStack(spacing: 4) {
                                    Text("最近探测")
                                    Text(Date(timeIntervalSince1970: Double(timestamp) / 1_000), style: .time)
                                }.font(.caption).foregroundStyle(.secondary)
                            }
                            if let error = route.probeError {
                                Text("探测失败：\(error)")
                                    .font(.caption).foregroundStyle(.secondary)
                                    .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                            }
                            if route.preferred {
                                Text("下次优先候选 · 最终选路以实际可达性为准")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading).padding(14)
                        if route.id != routes.last?.id { Divider().padding(.horizontal, 14) }
                    }
                }
                .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 12))
                if routes.count == 1 {
                    Text("目前仅发现这一条候选地址，不代表设备只有一个网络接口。")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            Text("同一地址只显示一次。自动模式按实测握手延迟选择最快可达线路；雷电/有线/Wi-Fi 优先会先走对应接口，不可用时再回退。已连接时每 30 秒后台重测备选线路，刷新可立即重测。探测不打断当前连接；发现来源不等于物理接口类型。")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}
