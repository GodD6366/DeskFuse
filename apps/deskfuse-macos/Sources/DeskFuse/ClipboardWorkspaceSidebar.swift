import SwiftUI
import DeskFuseEngine

private struct WorkflowSyncCard: View {
    @Bindable private var workflows = WorkflowStore.shared

    var body: some View {
        Button {
            AppStore.shared.openSettings(tab: "workflows")
        } label: {
            HStack {
                Image(systemName: "square.stack.3d.up")
                    .font(.system(size: 13, weight: .medium))
                Text("工作流")
                    .font(.system(size: 13, weight: .medium))
                Spacer()
                Text(String(workflows.definitions.count))
                    .font(.system(size: 11))
                    .opacity(0.8)
                Image(systemName: "chevron.right")
                    .font(.system(size: 10, weight: .semibold))
                    .opacity(0.7)
            }
            .foregroundStyle(.white)
            .padding(.horizontal, 12)
            .frame(height: 36)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.blue, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
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
