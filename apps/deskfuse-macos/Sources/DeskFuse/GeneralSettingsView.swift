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
                    featureRow(symbol: "doc.on.clipboard", color: .blue,
                               title: "自动同步剪贴板", isOn: $clipboardEnabled, enabled: ready)
                    Divider()
                    featureRow(symbol: "rectangle.on.rectangle", color: .purple,
                               title: "键鼠共享", isOn: $inputSharingEnabled, enabled: ready)
                    if let remoteApplicationEnabled {
                        Divider()
                        featureRow(symbol: "square.grid.2x2", color: .teal,
                                   title: "远程应用", isOn: remoteApplicationEnabled,
                                   enabled: true, beta: remoteApplicationBeta)
                    }
                    if let lanWebClient {
                        Divider()
                        LanWebFeatureToggle(client: lanWebClient)
                    }
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
                    settingsToggle("保持电脑唤醒", isOn: $keepAwake)
                    Divider()
                    Text("隐藏后只保留菜单栏图标；仍可从菜单栏打开主窗口和设置。\n保持唤醒将阻止系统自动休眠和显示器休眠，关闭应用时自动恢复。")
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
                    Text("日志默认开启，按小时保存，自动删除超过 7 天的日志；导出诊断会附带当前进程、OCR helper 的 RSS/physical footprint 和已链接框架。\n关闭后仍记录错误；不包含剪贴板正文、文件内容或凭据。磁盘日志位于 ~/Library/Logs/DeskFuse。")
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

    /// 功能总开关行：16pt 彩色符号 + 名称 + 右置开关，与各功能页同源绑定。
    private func featureRow(symbol: String, color: Color, title: String,
                            isOn: Binding<Bool>, enabled: Bool, beta: Bool = false) -> some View {
        HStack(spacing: 16) {
            Image(systemName: symbol)
                .font(.system(size: 16))
                .foregroundStyle(color)
                .frame(width: 24)
                .accessibilityHidden(true)
            Text(title)
            if beta { FeatureBadge() }
            Spacer(minLength: 16)
            Toggle(title, isOn: isOn)
                .labelsHidden()
                .toggleStyle(.switch)
                .disabled(!enabled)
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

struct SettingsGroupCard<Content: View>: View {
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
                .modifier(NativeGlassCardSurface(cornerRadius: 16))
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
        HStack(spacing: 16) {
            Image(systemName: "qrcode")
                .font(.system(size: 16))
                .foregroundStyle(.orange)
                .frame(width: 24)
                .accessibilityHidden(true)
            Text("局域网扫码分享")
            Spacer(minLength: 16)
            if configuring { ProgressView().controlSize(.small) }
            Toggle("局域网扫码分享", isOn: Binding(
                get: { state?.enabled ?? false },
                set: { configure(enabled: $0) }
            ))
            .labelsHidden()
            .toggleStyle(.switch)
            .disabled(state == nil || configuring)
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
