import SwiftUI

struct ErrorNoticeView: View {
    let notice: ErrorNotice

    var body: some View {
        if let message = notice.message {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                Text(message).textSelection(.enabled)
                Spacer(minLength: 8)
                Button(action: notice.dismiss) {
                    Image(systemName: "xmark")
                        .frame(width: 24, height: 24)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("关闭异常提示")
                .help("关闭异常提示")
            }
            .font(.callout)
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(.orange.opacity(0.28), lineWidth: 1)
            }
            .shadow(color: .black.opacity(0.16), radius: 12, y: 5)
        }
    }
}
