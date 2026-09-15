import SwiftUI

/// Display-only label. The owning row presents the editor even when no label
/// is rendered (for example an image card without a custom title).
struct ClipboardTitleLabel: View {
    let title: String?
    let fallback: String
    @Binding var editing: Bool
    var font: Font = .system(size: 13, weight: .semibold)

    private var customTitle: String { (title ?? "").trimmingCharacters(in: .whitespacesAndNewlines) }
    private var display: String { customTitle.isEmpty ? fallback : customTitle }

    var body: some View {
        if !display.isEmpty {
            Text(display)
                .font(font)
                .foregroundStyle(customTitle.isEmpty ? Color.secondary : Color.primary)
                .lineLimit(1)
                .truncationMode(.middle)
                .contentShape(Rectangle())
                .onTapGesture(count: 2) { editing = true }
                .help("双击编辑标题")
        }
    }
}

struct ClipboardTitleEditor: View {
    @Environment(\.dismiss) private var dismiss
    @FocusState private var focused: Bool
    @State private var draft: String
    private let initialTitle: String
    private let onSave: (String) -> Void

    init(title: String?, onSave: @escaping (String) -> Void) {
        initialTitle = title ?? ""
        _draft = State(initialValue: title ?? "")
        self.onSave = onSave
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("编辑标题").font(.title2.bold())
            TextField("标题", text: $draft)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit(save)
            Text("留空并保存可移除自定义标题，不会修改剪贴板内容。")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("取消") { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("保存", action: save)
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.borderedProminent)
            }
        }
        .padding(24)
        .frame(width: 360)
        .onAppear { focused = true }
    }

    private func save() {
        let value = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if value != initialTitle { onSave(value) }
        dismiss()
    }
}
