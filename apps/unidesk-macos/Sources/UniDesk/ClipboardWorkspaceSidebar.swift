import SwiftUI
import UniDeskEngine

/// The clipboard page has its own content navigation. This keeps the global
/// sidebar focused on destinations while the list of saved clipboard groups
/// stays next to the history it controls.
struct ClipboardWorkspaceSidebar: View {
    let snapshot: EngineSnapshot
    @Binding var selectedListID: String?
    let total: Int

    @Bindable private var library = ClipboardLibraryStore.shared

    private var imageCount: Int {
        snapshot.history.filter(\.hasImage).count
    }

    private var fileCount: Int {
        snapshot.history.filter { $0.fileCount > 0 }.count
    }

    private var groupCounts: [String: Int] {
        var result = ["all": max(total, snapshot.history.count)]
        for list in library.lists {
            switch list.name {
            case "图片": result[list.id] = imageCount
            case "文件": result[list.id] = fileCount
            case "文字": result[list.id] = snapshot.history.filter { !$0.hasImage && $0.fileCount == 0 }.count
            default: break
            }
        }
        return result
    }

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
                vertical: true,
                counts: groupCounts
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
