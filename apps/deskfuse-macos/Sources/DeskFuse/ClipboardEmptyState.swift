import SwiftUI

/// 剪贴板历史为空或筛选无结果时的占位，菜单栏面板与主窗口共用同一套样式。
struct ClipboardEmptyState: View {
    let symbol: String
    let title: String
    let message: String
    var onClear: (() -> Void)?

    var body: some View {
        VStack(spacing: 10) {
            Image(systemName: symbol)
                .font(DFType.metric).foregroundStyle(.tertiary)
            Text(title).font(DFType.section)
            Text(message)
                .font(DFType.detail).foregroundStyle(.secondary).multilineTextAlignment(.center)
            if let onClear {
                Button("清除筛选", action: onClear)
            }
        }
        .frame(maxWidth: .infinity).padding(.horizontal, 15).padding(.vertical, 45)
    }
}
