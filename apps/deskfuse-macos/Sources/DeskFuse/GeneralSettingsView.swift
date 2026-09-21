import SwiftUI

struct GeneralSettingsView: View {
    @Binding var clipboardEnabled: Bool
    @Binding var debugLoggingEnabled: Bool
    @Binding var hideDockIcon: Bool
    let ready: Bool
    let exportingDiagnostics: Bool
    let exportDiagnostics: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                SettingsGroupCard("共享") {
                    settingsToggle("自动同步剪贴板", isOn: $clipboardEnabled, enabled: ready)
                    Divider()
                    Text("拖拽文件时晃动鼠标可呼出暂存区。\n关闭主窗口后继续共享，退出 DeskFuse 时停止。")
                        .font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                SettingsGroupCard("系统") {
                    LoginItemSettingsView()
                    Divider()
                    settingsToggle("隐藏 Dock 图标", isOn: $hideDockIcon)
                    Divider()
                    Text("隐藏后只保留菜单栏图标；仍可从菜单栏打开主窗口和设置。")
                        .font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                SettingsGroupCard("诊断") {
                    settingsToggle("记录 Debug 日志", isOn: $debugLoggingEnabled, enabled: ready)
                    Divider()
                    HStack {
                        Text("导出诊断")
                        Spacer(minLength: 16)
                        WorkspaceChromeButton(exportingDiagnostics ? "正在导出…" : "导出…", systemImage: "square.and.arrow.up", action: exportDiagnostics)
                            .disabled(!ready || exportingDiagnostics)
                    }
                    Divider()
                    Text("日志默认开启，按小时保存，自动删除超过 7 天的日志；开关仅本次运行有效。\n关闭后仍记录错误；不包含剪贴板正文、文件内容或凭据。磁盘日志位于 ~/Library/Logs/DeskFuse。")
                        .font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: 560, alignment: .leading)
                }
            }
            .font(.system(size: 15))
            .frame(maxWidth: 680, alignment: .leading)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 36).padding(.bottom, 32)
        }
    }

    private func settingsToggle(_ title: String, isOn: Binding<Bool>, enabled: Bool = true) -> some View {
        HStack(spacing: 16) {
            Text(title)
            Spacer(minLength: 16)
            Toggle(title, isOn: isOn)
                .labelsHidden().toggleStyle(.switch).controlSize(.small)
                .disabled(!enabled)
        }
        .frame(minHeight: WorkspaceControlMetrics.height)
    }
}

private struct SettingsGroupCard<Content: View>: View {
    let title: String
    @ViewBuilder let content: () -> Content

    init(_ title: String, @ViewBuilder content: @escaping () -> Content) {
        self.title = title
        self.content = content
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title).font(.system(size: 15, weight: .semibold)).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 12, content: content)
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 16))
                .overlay { RoundedRectangle(cornerRadius: 16).strokeBorder(.quaternary, lineWidth: 1) }
        }
    }
}
