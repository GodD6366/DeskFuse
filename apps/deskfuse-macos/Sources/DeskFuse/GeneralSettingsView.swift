import SwiftUI

struct GeneralSettingsView: View {
    @Binding var clipboardEnabled: Bool
    @Binding var inputSharingEnabled: Bool
    /// macOS 14.2 以下没有远程应用，传 nil 隐藏该行。
    var remoteApplicationEnabled: Binding<Bool>?
    var remoteApplicationBeta = false
    var lanWebClient: EngineClient?
    @Binding var debugLoggingEnabled: Bool
    @Binding var hideDockIcon: Bool
    @Binding var keepAwake: Bool
    let ready: Bool
    let exportingDiagnostics: Bool
    let exportDiagnostics: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                SettingsGroupCard("功能") {
                    featureRow(symbol: "doc.on.clipboard", title: "自动同步剪贴板",
                               detail: "关闭主窗口后继续共享，退出 DeskFuse 时停止。",
                               isOn: $clipboardEnabled, enabled: ready)
                    Divider()
                    featureRow(symbol: "rectangle.on.rectangle",
                               title: "键鼠共享", isOn: $inputSharingEnabled, enabled: ready)
                    if let remoteApplicationEnabled {
                        Divider()
                        featureRow(symbol: "square.grid.2x2",
                                   title: "远程应用", isOn: remoteApplicationEnabled,
                                   enabled: true, beta: remoteApplicationBeta)
                    }
                    if let lanWebClient {
                        Divider()
                        LanWebFeatureToggle(client: lanWebClient)
                    }
                    Divider()
                    QuietSettingsRow(title: "文件暂存区", detail: "拖拽文件时晃动鼠标可呼出暂存区。", symbol: "tray") {
                        EmptyView()
                    }
                }
                SettingsGroupCard("系统") {
                    LoginItemSettingsView()
                    Divider()
                    QuietSettingsToggle("隐藏 Dock 图标",
                        detail: "隐藏后只保留菜单栏图标；仍可从菜单栏打开主窗口和设置。", isOn: $hideDockIcon)
                    Divider()
                    QuietSettingsToggle("保持电脑唤醒",
                        detail: "阻止系统和显示器自动休眠，关闭应用时自动恢复。", isOn: $keepAwake)
                }
                SettingsGroupCard("诊断") {
                    QuietSettingsToggle("记录 Debug 日志",
                        detail: "日志默认开启，按小时保存，自动删除超过 7 天的记录；关闭后仍记录错误。\n不包含剪贴板正文、文件内容或凭据。保存位置：~/Library/Logs/DeskFuse。",
                        isOn: $debugLoggingEnabled)
                        .disabled(!ready)
                    Divider()
                    QuietSettingsRow(title: "导出诊断",
                        detail: "附带当前进程、OCR helper 的 RSS / physical footprint 和已链接框架。") {
                        WorkspaceChromeButton(exportingDiagnostics ? "正在导出…" : "导出…", systemImage: "square.and.arrow.up", action: exportDiagnostics)
                            .disabled(!ready || exportingDiagnostics)
                    }
                }
            }
            .font(.system(size: 13))
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
            .padding(.bottom, SystemSettingsMetrics.formPageBottomInset)
        }
    }

    /// Functional toggles share the same monochrome row and control dimensions.
    private func featureRow(symbol: String, title: String, detail: String? = nil,
                            isOn: Binding<Bool>, enabled: Bool, beta: Bool = false) -> some View {
        QuietSettingsRow(title: title, detail: detail, symbol: symbol, badge: beta ? "Beta" : nil) {
            Toggle(title, isOn: isOn)
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.small)
                .disabled(!enabled)
        }
    }

}

struct SettingsGroupCard<Content: View>: View {
    @Environment(\.quietWorkspace) private var quiet
    let title: String
    @ViewBuilder let content: () -> Content

    init(_ title: String, @ViewBuilder content: @escaping () -> Content) {
        self.title = title
        self.content = content
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            SystemSettingsSectionLabel(title: title)
            VStack(alignment: .leading, spacing: quiet ? 6 : 12, content: content)
                .padding(quiet ? 14 : SystemSettingsMetrics.blockPadding)
                .frame(maxWidth: .infinity, alignment: .leading)
                .systemSettingsBlock()
        }
    }
}

/// 局域网扫码分享总开关：状态在引擎侧持久化，进入设置页时读取当前值。
private struct LanWebFeatureToggle: View {
    let client: EngineClient
    @State private var state: Projection?
    @State private var configuring = false

    /// 只投影开关需要的字段；完整状态仍在「局域网传输」页展示。
    private struct Projection: Decodable {
        let enabled: Bool
        let port: UInt16?
        let preferredInterface: String?
    }

    var body: some View {
        QuietSettingsRow(title: "局域网扫码分享", symbol: "qrcode") {
            HStack(spacing: 8) {
                if configuring { ProgressView().controlSize(.small) }
                Toggle("局域网扫码分享", isOn: Binding(
                    get: { state?.enabled ?? false },
                    set: { configure(enabled: $0) }
                ))
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.small)
                .disabled(state == nil || configuring)
            }
        }
        .task { await refresh() }
    }

    private func refresh() async {
        guard let json = try? await client.lanWebState() else { return }
        state = try? JSONDecoder().decode(Projection.self, from: Data(json.utf8))
    }

    private func configure(enabled: Bool) {
        configuring = true
        Task {
            defer { configuring = false }
            let json = try? await client.configureLanWeb(
                enabled: enabled,
                interfaceName: state?.preferredInterface,
                port: state?.port ?? 0
            )
            if let json {
                state = try? JSONDecoder().decode(Projection.self, from: Data(json.utf8))
            }
        }
    }
}
