import AppKit
import SwiftUI
import UniDeskEngine

struct SettingsPermissionsView: View {
    let store: AppStore
    let permissions: [Permission]

    var body: some View {
        Form {
            Section {
                ForEach(permissions, id: \.name) { permission in
                    permissionRow(permission)
                }
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
                Text("状态来自 macOS。输入监控仅用于诊断，不作为键鼠共享的额外门槛。")
            }
        }
        .formStyle(.grouped)
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
        case .accessibility: "读取 Finder 粘贴位置与控制键鼠"
        case .fullDiskAccess: "访问受系统保护的文件与目录"
        case .inputMonitoring: "诊断本机键盘与鼠标事件"
        }
    }
}
