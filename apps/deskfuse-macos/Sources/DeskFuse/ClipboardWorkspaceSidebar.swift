import SwiftUI
import DeskFuseEngine

private struct WorkflowSyncCard: View {
    @Bindable private var workflows = WorkflowStore.shared

    var body: some View {
        Button {
            AppStore.shared.openSettings(tab: "clipboard", clipboardSegment: "workflows")
        } label: {
            HStack(spacing: 9) {
                Image(systemName: "square.stack.3d.up")
                    .font(.system(size: 14, weight: .semibold))
                    .frame(width: SystemSettingsMetrics.iconSize, height: SystemSettingsMetrics.iconSize)
                    .foregroundStyle(.secondary)
                Text("工作流")
                    .font(.system(size: 13))
                Spacer(minLength: 0)
                Text(String(workflows.definitions.count))
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                Image(systemName: "chevron.right")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 10)
            .frame(height: 36)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .frame(maxWidth: .infinity, alignment: .leading)
        .help("在设置中管理工作流")
    }
}

/// The clipboard page has its own content navigation. This keeps the global
/// sidebar focused on destinations while the list of saved clipboard groups
/// stays next to the history it controls.
struct ClipboardWorkspaceSidebar: View {
    @Binding var selectedListID: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("分组与列表")
                .font(WorkspaceTypography.heading)
                .padding(.horizontal, 20)
                .frame(height: 56)

            ScrollView {
                ClipboardLibraryTabs(selectedListID: $selectedListID, vertical: true)
                    .padding(.horizontal, 10)
            }

            Spacer(minLength: 0)

            WorkflowSyncCard()
                .padding(12)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        // 这一栏和它右侧的历史列表同属内容区，用同一个亮底，靠 ClipboardView
        // 的 Divider 分栏；只有最外层的主侧栏用材质灰底。
        .quietWorkspaceStyle()
    }
}
