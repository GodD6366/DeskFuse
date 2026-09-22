import SwiftUI

struct ErrorNoticeView: View {
    let notice: ErrorNotice

    var body: some View {
        if let message = notice.message {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "exclamationmark.triangle")
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
            .background(.yellow.opacity(0.12))
        }
    }
}
