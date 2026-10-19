import AppKit
import SwiftUI

/// 常驻错误条：可操作的错误留到用户处理或关闭，原文只在「复制详情」里。
struct ErrorNoticeView: View {
    let notice: ErrorNotice
    var width: CGFloat? = nil

    var body: some View {
        if let message = notice.message {
            DFNotice(tone: .warning,
                     title: "操作未完成",
                     message: message,
                     actionTitle: "复制详情",
                     action: { copyDetails() },
                     onDismiss: notice.dismiss)
                .frame(width: width)
                .accessibilityElement(children: .contain)
        }
    }

    private func copyDetails() {
        let details = notice.rawMessage ?? notice.message ?? ""
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(details, forType: .string)
    }
}
