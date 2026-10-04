import AppKit
import SwiftUI
import DeskFuseRemoteApplication
import DeskFuseEngine

struct SettingsPermissionsView: View {
    let store: AppStore
    let permissions: [Permission]
    @Environment(\.scenePhase) private var scenePhase
    @State private var screenRecordingGranted = false
    @State private var wifiGranted = false

    var body: some View {
        Form {
            Section {
                ForEach(permissions, id: \.name) { permission in
                    permissionRow(permission)
                }
                if #available(macOS 14.2, *) {
                    screenRecordingRow
                }
                wifiRow
                if permissions.isEmpty {
                    Text("正在读取系统权限…").foregroundStyle(.secondary)
                }
            } header: {
                HStack {
                    Text("系统权限")
                    Spacer()
                    Button { store.send(.refreshPermissions) } label: {
                        Label("刷新", systemImage: "arrow.clockwise")
                    }
                    .buttonStyle(.borderless)
                }
            } footer: {
                HStack {
                    Text("状态来自 macOS。仅展示 DeskFuse 实际使用的系统权限。")
                    Spacer()
                    if store.permissionPrompt.hasSuppressedPermissions {
                        Button("恢复启动授权提醒") { store.permissionPrompt.resetSuppression() }
                            .buttonStyle(.borderless)
                    }
                }
            }
        }
        .formStyle(.grouped)
        .onAppear {
            refreshScreenRecordingPermission()
            refreshWifiPermission()
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active {
                refreshScreenRecordingPermission()
                refreshWifiPermission()
            }
        }
    }

    /// SSID reads power per-Wi-Fi controller and layout memory. The row lives on
    /// this page so the launch prompt and 设置 → 权限 offer the same request.
    private var wifiRow: some View {
        HStack(spacing: 16) {
            Image(systemName: "wifi")
                .font(.system(size: 20))
                .foregroundStyle(.secondary)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 5) {
                Text("Wi-Fi 名称（定位）").font(.body.weight(.medium))
                Text("读取当前 Wi-Fi 名称，用于按网络保存主控与布局；不采集位置")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Label(wifiGranted ? "已授权" : WifiPermission.statusText,
                  systemImage: wifiGranted ? "checkmark.circle.fill" : "circle.dashed")
                .font(.caption)
                .foregroundStyle(wifiGranted ? Color.green : Color.secondary)
                .frame(width: 120, alignment: .leading)
            Button(wifiGranted ? "查看设置" : "前往授权") {
                if wifiGranted {
                    WifiPermission.openSettings()
                } else {
                    WifiPermission.request()
                }
                refreshWifiPermission()
            }
            .buttonStyle(.bordered)
            .frame(width: 88)
        }
        .padding(.vertical, 8)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Wi-Fi 名称（定位），\(wifiGranted ? "已授权" : WifiPermission.statusText)")
    }

    private func refreshWifiPermission() {
        wifiGranted = WifiPermission.isGranted
    }

    @available(macOS 14.2, *)
    private var screenRecordingRow: some View {
        HStack(spacing: 16) {
            Image(systemName: "rectangle.dashed.badge.record")
                .font(.system(size: 20))
                .foregroundStyle(.secondary)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 5) {
                Text("屏幕录制").font(.body.weight(.medium))
                Text("远程应用访问本机窗口时捕获屏幕内容")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Label(screenRecordingGranted ? "已授权" : "未授权或需要操作",
                  systemImage: screenRecordingGranted ? "checkmark.circle.fill" : "circle.dashed")
                .font(.caption)
                .foregroundStyle(screenRecordingGranted ? Color.green : Color.secondary)
                .frame(width: 120, alignment: .leading)
            Button(screenRecordingGranted ? "查看设置" : "前往授权") {
                if screenRecordingGranted {
                    openScreenRecordingSettings()
                } else {
                    _ = RemoteAppCapture.requestScreenRecordingPermission()
                }
                refreshScreenRecordingPermission()
            }
            .buttonStyle(.bordered)
            .frame(width: 88)
        }
        .padding(.vertical, 8)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("屏幕录制，\(screenRecordingGranted ? "已授权" : "未授权")")
    }

    private func refreshScreenRecordingPermission() {
        guard #available(macOS 14.2, *) else { return }
        screenRecordingGranted = RemoteAppCapture.hasScreenRecordingPermission
    }

    private func openScreenRecordingSettings() {
        guard let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture") else { return }
        NSWorkspace.shared.open(url)
    }

    private func permissionRow(_ permission: Permission) -> some View {
        HStack(spacing: 16) {
            Image(systemName: symbol(permission.kind))
                .font(.system(size: 20))
                .foregroundStyle(.secondary)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 5) {
                Text(permission.name).font(.body.weight(.medium))
                Text(purpose(permission.kind)).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Label(permission.status,
                  systemImage: permission.granted ? "checkmark.circle.fill" : "circle.dashed")
                .font(.caption)
                .fixedSize(horizontal: false, vertical: true)
                .foregroundStyle(permission.granted ? Color.green : Color.secondary)
                .frame(width: 120, alignment: .leading)
                .help(permission.status)
                .accessibilityLabel(permission.status)
            Button(permission.granted ? "查看设置" : "前往授权") {
                if permission.granted {
                    if let url = URL(string: permission.settingsUrl) { NSWorkspace.shared.open(url) }
                } else {
                    do { try requestPermission(kind: permission.kind) }
                    catch { store.error = error.userFacingMessage }
                }
                store.send(.refreshPermissions)
            }
            .buttonStyle(.bordered)
            .frame(width: 88)
        }
        .padding(.vertical, 8)
    }

    private func symbol(_ kind: PermissionKind) -> String {
        switch kind {
        case .accessibility: "accessibility"
        case .fullDiskAccess: "folder.badge.gearshape"
        case .inputMonitoring: "keyboard"
        }
    }

    private func purpose(_ kind: PermissionKind) -> String {
        switch kind {
        case .accessibility: "拦截文件粘贴、检查 Finder 焦点与控制键鼠"
        case .fullDiskAccess: "访问受系统保护的文件与目录"
        case .inputMonitoring: "诊断本机键盘与鼠标事件"
        }
    }
}
