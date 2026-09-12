import SwiftUI
import UniDeskEngine

struct TransfersView: View {
    @Bindable var store: AppStore
    let transfers: [Transfer]

    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    HStack(alignment: .top, spacing: 20) {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("传输").font(.system(size: 26, weight: .bold))
                            Text("文件在目标设备粘贴时传输，也可以从暂存区立即发送。")
                                .font(.callout).foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 0)
                        Button("打开暂存区") { store.shelf?.show() }.controlSize(.large)
                    }
                    HStack(spacing: 0) {
                        summary("进行中", count: transfers.filter { $0.phase == .sending || $0.phase == .receiving }.count, color: .accentColor)
                        Divider().frame(height: 18)
                        summary("排队", count: transfers.filter { $0.phase == .queued }.count, color: .primary)
                        Divider().frame(height: 18)
                        summary("可重试", count: transfers.filter { $0.outgoing && ($0.phase == .failed || $0.phase == .cancelled) }.count, color: .orange)
                    }
                    .padding(.vertical, 16)
                    .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 14))
                    LazyVStack(spacing: 0) {
                        ForEach(transfers, id: \.id) { task in
                            TransferQueueRow(store: store, task: task, compact: geometry.size.width < 860)
                            Divider()
                        }
                    }
                    ContentUnavailableView(transfers.isEmpty ? "没有传输任务" : "没有其他传输任务",
                                           systemImage: "arrow.up.arrow.down",
                                           description: Text("在目标设备粘贴文件时会自动开始传输。"))
                        .frame(maxWidth: .infinity).padding(.vertical, 24)
                }
                .padding(28)
                .frame(maxWidth: .infinity, minHeight: geometry.size.height, alignment: .topLeading)
            }
            .background(Color(nsColor: .textBackgroundColor))
        }
    }

    private func summary(_ title: String, count: Int, color: Color) -> some View {
        HStack(spacing: 12) {
            Text(title).font(.callout).foregroundStyle(.secondary)
            Text("\(count)").font(.system(size: 18, weight: .semibold)).monospacedDigit().foregroundStyle(color)
        }.frame(maxWidth: .infinity)
    }
}

private struct TransferQueueRow: View {
    @Bindable var store: AppStore
    let task: Transfer
    let compact: Bool
    private var active: Bool { task.phase == .sending || task.phase == .receiving }
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
                    if task.outgoing { Button("重试") { store.send(.retryTransfer(id: task.id)) } }
                } else if task.phase != .completed {
                    Button("取消") { store.send(.cancelTransfer(id: task.id)) }
                }
            }.controlSize(.large).frame(width: 76)
        }.padding(.vertical, 22)
    }

    @ViewBuilder private var progress: some View {
        if active {
            VStack(alignment: .leading, spacing: 8) {
                if task.total > 0 {
                    HStack {
                        ProgressView(value: Double(min(task.transferred, task.total)), total: Double(task.total))
                        Text("\(Int(min(1, Double(task.transferred) / Double(task.total)) * 100))%")
                            .font(.caption).monospacedDigit().foregroundStyle(.secondary)
                    }
                    Text("\(byteCount(task.transferred)) / \(byteCount(task.total))")
                        .font(.caption).monospacedDigit().foregroundStyle(.secondary)
                } else {
                    HStack { ProgressView().controlSize(.small); Text("已传输 \(byteCount(task.transferred))") }
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
        } else if task.phase == .queued {
            HStack(spacing: 10) { ProgressView().controlSize(.small); Text("等待传输…").font(.caption).foregroundStyle(.secondary) }
        } else if task.phase == .completed {
            Label("已完成", systemImage: "checkmark.circle").font(.caption).foregroundStyle(.secondary)
        } else if task.phase == .cancelled {
            Text("已取消").font(.caption).foregroundStyle(.secondary)
        }
    }
}
