import SwiftUI

struct SettingsSaveFooter: View {
    var dirty: Bool
    var saving: Bool
    var canSave: Bool
    var canRevert: Bool
    var status: String?
    var error: String?
    var revert: () -> Void
    var save: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Divider()
            HStack {
                if saving {
                    ProgressView().controlSize(.small)
                    Text("保存中…").font(.caption).foregroundStyle(.secondary)
                } else if dirty {
                    Text("未保存").font(.caption).foregroundStyle(.secondary)
                } else if let status {
                    Text(status).font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                Button("撤销修改", action: revert).disabled(!canRevert || saving)
                Button(saving ? "保存中…" : "保存", action: save)
                    .buttonStyle(.borderedProminent).disabled(!canSave || saving)
            }.padding(.horizontal, 12)
            if let error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 12)
            }
        }.padding(.bottom, 12)
    }
}
