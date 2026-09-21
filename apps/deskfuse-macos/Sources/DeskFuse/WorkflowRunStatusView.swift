import SwiftUI
import DeskFuseEngine

struct WorkflowRunStatusView: View {
    @Bindable private var workflows = WorkflowStore.shared
    var compact = false
    private var visible: [WorkflowRun] { Array(workflows.runs.prefix(compact ? 1 : 3)) }
    var body: some View {
        if !visible.isEmpty || workflows.error != nil {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(visible, id: \.id) { run in
                    HStack(alignment: .top, spacing: 10) {
                        if run.phase == "running" { ProgressView().controlSize(.small) }
                        else { Image(systemName: icon(run.phase)).foregroundStyle(run.phase == "failed" ? Color.orange : Color.secondary) }
                        VStack(alignment: .leading, spacing: 3) {
                            Text(title(run)).font(.caption.bold()).lineLimit(1)
                            if !run.message.isEmpty { Text(run.message).font(.caption).foregroundStyle(.secondary).lineLimit(compact ? 2 : 4) }
                            if run.phase == "completed", run.outputBytes > 0 {
                                // Keep metadata in SwiftUI text rendering. Selectable Text
                                // introduces an AppKit text view with inverted glyphs here.
                                VStack(alignment: .leading, spacing: 2) {
                                    if run.outputKind == .image && run.width > 0 && run.height > 0 {
                                        Text(verbatim: "\(formatName(run.outputMime)) · \(run.width) × \(run.height) px")
                                    } else {
                                        Text(run.outputKind == .text ? "文字结果" : "\(run.outputPaths.count) 个文件或文件夹")
                                    }
                                    Text(verbatim: "\(byteCount(run.inputBytes)) → \(byteCount(run.outputBytes))")
                                        .monospacedDigit()
                                }
                                .font(.caption).foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                            }
                        }.frame(maxWidth: .infinity, alignment: .leading)
                        if run.phase == "running" {
                            Button("取消") { workflows.cancel(run.id) }.controlSize(.small)
                        }
                    }
                }
                if let error = workflows.error { Text(error).font(.caption).foregroundStyle(.red).lineLimit(compact ? 2 : 4) }
            }.padding(compact ? 4 : 12)
        }
    }
    private func formatName(_ mime: String) -> String {
        switch mime {
        case "image/jpeg": "JPEG"
        case "image/png": "PNG"
        case "image/webp": "WebP"
        case "image/heic": "HEIC"
        default: mime
        }
    }
    private func title(_ run: WorkflowRun) -> String {
        let name = workflows.definitions.first { $0.id == run.workflowId }?.name ?? "工作流"
        switch run.phase {
        case "running": return "\(name) · 第 \(run.stepIndex + 1) 步"
        case "completed": return "\(name) · 已完成"
        case "cancelled": return "\(name) · 已取消"
        case "failed": return "\(name) · 失败"
        default: return name
        }
    }
    private func icon(_ phase: String) -> String {
        switch phase {
        case "completed": "checkmark.circle"
        case "cancelled": "xmark.circle"
        default: "exclamationmark.triangle"
        }
    }
}
