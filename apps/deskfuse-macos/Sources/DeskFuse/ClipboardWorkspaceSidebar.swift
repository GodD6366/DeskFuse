import SwiftUI
import DeskFuseEngine

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

            VStack(alignment: .leading, spacing: 8) {
                Label("自动同步", systemImage: "arrow.triangle.2.circlepath")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text("历史会在已配对设备间保持同步")
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(nsColor: .controlBackgroundColor).opacity(0.45), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .padding(12)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(nsColor: .textBackgroundColor))
    }
}
