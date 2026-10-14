import AppKit
import SwiftUI
import DeskFuseRemoteApplication
import DeskFuseEngine

struct SettingsPermissionsView: View {
    let store: AppStore
    let permissions: [Permission]
    @Environment(\.scenePhase) private var scenePhase
    @State private var screenRecordingGranted = false

    var body: some View {
        SystemSettingsFormPage {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 12) {
                    SystemSettingsSectionLabel(title: "系统权限")
                    Spacer(minLength: 0)
                    Button {
                        store.send(.refreshPermissions)
                        refreshScreenRecordingPermission()
                    } label: {
                        Label("刷新", systemImage: "arrow.clockwise")
                    }
                    .buttonStyle(.borderless)
                    .controlSize(.small)
                }
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(permissions, id: \.name) { permission in
                        permissionRow(permission)
                        Divider()
                    }
                    if #available(macOS 14.2, *) {
                        screenRecordingRow
                        Divider()
                    }
                    wifiPermissionRow
                    if permissions.isEmpty {
                        Text("正在读取系统权限…").foregroundStyle(.secondary)
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 6)
                .frame(maxWidth: .infinity, alignment: .leading)
                .systemSettingsBlock()
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    Text("状态来自 macOS。仅展示 DeskFuse 实际使用的系统权限。")
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                    if store.permissionPrompt.hasSuppressedPermissions {
                        Button("恢复启动授权提醒") { store.permissionPrompt.resetSuppression() }
                            .buttonStyle(.borderless)
                            .font(.caption)
                    }
                }
                .padding(.horizontal, 4)
            }
        }
        .onAppear {
            refreshScreenRecordingPermission()
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active {
                refreshScreenRecordingPermission()
            }
        }
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
            .buttonStyle(WorkspaceButtonStyle())
            .frame(width: 88)
        }
        .padding(.vertical, 8)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("屏幕录制，\(screenRecordingGranted ? "已授权" : "未授权")")
    }

    private var wifiPermissionRow: some View {
        let state = store.permissionPrompt.wifiState
        let granted = state == .granted
        return HStack(spacing: 16) {
            Image(systemName: "wifi").font(.system(size: 20)).foregroundStyle(.secondary).frame(width: 28)
            VStack(alignment: .leading, spacing: 5) {
                Text(LaunchPermission.wifi.title).font(.body.weight(.medium))
                Text(LaunchPermission.wifi.purpose).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }.frame(maxWidth: .infinity, alignment: .leading)
            Label(state.label, systemImage: granted ? "checkmark.circle.fill" : "circle.dashed")
                .font(.caption).foregroundStyle(granted ? Color.green : Color.secondary)
                .frame(width: 120, alignment: .leading)
            Button(granted ? "查看设置" : "前往授权") { store.permissionPrompt.authorize(.wifi) }
                .buttonStyle(WorkspaceButtonStyle()).frame(width: 88)
        }.padding(.vertical, 8)
            .accessibilityElement(children: .combine)
            .accessibilityLabel("\(LaunchPermission.wifi.title)，\(state.label)")
    }

    private func refreshScreenRecordingPermission() {
        store.permissionPrompt.refreshWifiPermission()
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
            .buttonStyle(WorkspaceButtonStyle())
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
