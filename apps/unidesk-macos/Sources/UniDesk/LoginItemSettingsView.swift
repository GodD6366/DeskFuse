import AppKit
import ServiceManagement
import SwiftUI

/// The system login item is the source of truth; no separate preference is stored.
@MainActor
struct LoginItemSettingsView: View {
    @State private var status = SMAppService.mainApp.status
    @State private var updating = false
    @State private var errorMessage: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 16) {
                Text("开机自启")
                Spacer(minLength: 16)
                Toggle("开机自启", isOn: Binding(
                    get: { status == .enabled || status == .requiresApproval },
                    set: { setEnabled($0) }
                ))
                .labelsHidden().toggleStyle(.switch).controlSize(.small)
                .disabled(updating)
            }
            .frame(minHeight: WorkspaceControlMetrics.height)

            Text(statusDescription)
                .font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            if status == .requiresApproval {
                WorkspaceChromeButton("打开登录项设置", systemImage: "gearshape") {
                    SMAppService.openSystemSettingsLoginItems()
                }
            }
            if let errorMessage {
                Text(errorMessage)
                    .font(.callout).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .onAppear { refreshStatus() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            refreshStatus()
        }
    }

    private var statusDescription: String {
        switch status {
        case .enabled:
            "已开启，登录 Mac 后自动启动 uniDesk。"
        case .requiresApproval:
            "等待系统批准，请在系统设置的登录项中允许 uniDesk。"
        case .notRegistered:
            "登录 Mac 后自动启动 uniDesk；更改立即生效。"
        case .notFound:
            "系统未找到登录项，请将 uniDesk 安装到“应用程序”后重试。"
        @unknown default:
            "无法确认登录项状态，请在系统设置中检查。"
        }
    }

    private func refreshStatus() {
        status = SMAppService.mainApp.status
    }

    private func setEnabled(_ enabled: Bool) {
        guard !updating else { return }
        updating = true
        errorMessage = nil
        Task { @MainActor in
            defer {
                refreshStatus()
                updating = false
            }
            do {
                if enabled {
                    try SMAppService.mainApp.register()
                } else {
                    try await SMAppService.mainApp.unregister()
                }
            } catch {
                errorMessage = "无法\(enabled ? "开启" : "关闭")开机自启：\(error.localizedDescription)"
            }
        }
    }
}
