import SwiftUI

/// A lazy-list sentinel, not a timer: only requests the next page while visible.
/// Remember the attempted cursor so an error never becomes an automatic retry loop.
struct ClipboardPaginationTrigger: View {
    let itemCount: Int
    let searching: Bool
    let loadMore: () -> Void
    @State private var visible = false
    @State private var attemptedCursor: Int?

    var body: some View {
        Group {
            if searching {
                ProgressView().controlSize(.small).accessibilityLabel("正在加载历史")
            } else if attemptedCursor == itemCount {
                Button("加载未完成，重试") { attemptedCursor = nil; requestIfNeeded() }
                    .buttonStyle(.plain)
            } else {
                Text("继续滚动以加载历史").font(.caption).foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, minHeight: 36)
        .onAppear { visible = true; requestIfNeeded() }
        .onDisappear { visible = false }
        .onChange(of: itemCount) { _, _ in requestIfNeeded() }
        .onChange(of: searching) { _, _ in requestIfNeeded() }
    }

    private func requestIfNeeded() {
        guard visible, !searching, attemptedCursor != itemCount else { return }
        attemptedCursor = itemCount
        loadMore()
    }
}
