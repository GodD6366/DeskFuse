import SwiftUI
import DeskFuseEngine

private struct WorkflowSyncCard: View {
    @Bindable private var workflows = WorkflowStore.shared
    @State private var isExpanded = false
    
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                withAnimation(.spring(duration: 0.25)) {
                    isExpanded.toggle()
                }
            } label: {
                HStack {
                    Image(systemName: "square.stack.3d.up")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text("工作流")
                        .font(.caption.weight(.medium))
                    Spacer()
                    Text(String(workflows.definitions.count))
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(isExpanded ? 90 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            
            if isExpanded {
                VStack(alignment: .leading, spacing: 6) {
                    Label(syncTitle(workflows.catalog.sync_status), systemImage: "arrow.triangle.2.circlepath")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                    
                    if workflows.definitions.isEmpty {
                        Text("尚未保存流程")
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                    } else {
                        ForEach(workflows.definitions.prefix(3)) { flow in
                            HStack(spacing: 6) {
                                Circle()
                                    .fill(flow.enabled ? Color.green : Color.secondary)
                                    .frame(width: 4, height: 4)
                                Text(flow.name)
                                    .font(.caption2)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(1)
                            }
                        }
                        
                        if workflows.definitions.count > 3 {
                            Text("及其他 \(workflows.definitions.count - 3) 个")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        }
                    }
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(nsColor: .controlBackgroundColor).opacity(0.45), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
    
    private func syncTitle(_ status: String) -> String {
        switch status {
        case "synced": "同步完成"
        case "pending": "等待同步"
        case "waiting_for_devices": "等待设备连接"
        case "error": "同步失败"
        default: status
        }
    }
}

/// The clipboard page has its own content navigation. This keeps the global
/// sidebar focused on destinations while the list of saved clipboard groups
/// stays next to the history it controls.
struct ClipboardWorkspaceSidebar: View {
    @Binding var selectedListID: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 7) {
                Text("剪贴板")
                    .font(.system(size: 21, weight: .bold))
                Text("在所有设备间，自由流转")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            .padding(.horizontal, 20)
            .padding(.top, 24)
            .padding(.bottom, 18)

            ClipboardLibraryTabs(
                selectedListID: $selectedListID,
                vertical: true
            )
            .padding(.horizontal, 10)

            Spacer(minLength: 0)

            WorkflowSyncCard()
                .padding(12)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(nsColor: .textBackgroundColor))
    }
}
