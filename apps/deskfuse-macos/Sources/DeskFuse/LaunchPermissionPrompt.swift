import AppKit
import Observation
import SwiftUI
import PermissionFlow
import PermissionFlowScreenRecordingStatus

/// The three system permissions highlighted during launch. The order matches
/// the launch prompt and the settings permissions page.
enum LaunchPermission: String, CaseIterable, Identifiable {
    case accessibility
    case fullDiskAccess
    case screenRecording

    var id: String { rawValue }

    var pane: PermissionFlowPane {
        switch self {
        case .accessibility: .accessibility
        case .fullDiskAccess: .fullDiskAccess
        case .screenRecording: .screenRecording
        }
    }

    var title: String {
        switch self {
        case .accessibility: "辅助功能"
        case .fullDiskAccess: "完全磁盘访问"
        case .screenRecording: "屏幕录制"
        }
    }

    var purpose: String {
        switch self {
        case .accessibility: "拦截文件粘贴、检查 Finder 焦点与控制键鼠"
        case .fullDiskAccess: "访问受系统保护的文件与目录"
        case .screenRecording: "远程应用访问本机窗口时捕获屏幕内容"
        }
    }

    var symbol: String {
        switch self {
        case .accessibility: "accessibility"
        case .fullDiskAccess: "folder.badge.gearshape"
        case .screenRecording: "rectangle.dashed.badge.record"
        }
    }
}

/// Owns the once-per-launch permission prompt. Status comes from
/// PermissionFlow's official-API providers, so probing never triggers a
/// system alert; suppression is per permission and is lifted again once the
/// grant is in place, so a later revocation brings the prompt back.
@MainActor
@Observable
final class PermissionPromptController {
    /// Permissions currently listed in the launch prompt; `nil` hides it.
    var presented: [LaunchPermission]?

    static let suppressionDefaultsKey = "deskfuse.permissionPrompt.suppressed"

    private var evaluatedThisLaunch = false
    private let defaults: UserDefaults
    private(set) var suppressedPermissions: Set<LaunchPermission>

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        suppressedPermissions = Set(
            (defaults.stringArray(forKey: Self.suppressionDefaultsKey) ?? [])
                .compactMap(LaunchPermission.init(rawValue:))
        )
    }

    /// Registers screen-recording status detection and shows the prompt when
    /// grants are still missing. Called once at launch;
    /// already-granted or fully suppressed launches stay silent.
    func evaluateAtLaunch() {
        PermissionFlowScreenRecordingStatus.register()
        guard !evaluatedThisLaunch else { return }
        evaluatedThisLaunch = true
        liftSuppressionForGranted()
        let pending = pendingPermissions()
        presented = pending.isEmpty ? nil : pending
    }

    /// Called when the app regains focus while the prompt is visible: grants
    /// made in System Settings collapse the list, and an empty list closes it.
    func reevaluateWhilePresented() {
        guard presented != nil else { return }
        liftSuppressionForGranted()
        let pending = pendingPermissions()
        presented = pending.isEmpty ? nil : pending
    }

    /// Applies the "不再提醒" choice to the permissions listed right now.
    func suppressPresented() {
        guard let presented else { return }
        suppress(presented)
        self.presented = nil
    }

    /// Clears every suppression; the next launch prompts for what is missing.
    func resetSuppression() {
        persistSuppressedPermissions([])
    }

    var hasSuppressedPermissions: Bool {
        !suppressedPermissions.isEmpty
    }

    private func pendingPermissions() -> [LaunchPermission] {
        LaunchPermission.allCases.filter { !isGranted($0) && !suppressedPermissions.contains($0) }
    }

    private func suppress(_ permissions: [LaunchPermission]) {
        guard !permissions.isEmpty else { return }
        persistSuppressedPermissions(suppressedPermissions.union(permissions))
    }

    /// A permission that is granted again is no longer suppressed, so a later
    /// revocation brings the prompt back instead of staying silent forever.
    private func liftSuppressionForGranted() {
        let granted = LaunchPermission.allCases.filter(isGranted)
        guard !granted.isEmpty else { return }
        persistSuppressedPermissions(suppressedPermissions.subtracting(granted))
    }

    private func persistSuppressedPermissions(_ permissions: Set<LaunchPermission>) {
        suppressedPermissions = permissions
        if permissions.isEmpty {
            defaults.removeObject(forKey: Self.suppressionDefaultsKey)
        } else {
            defaults.set(
                permissions.map(\.rawValue).sorted(),
                forKey: Self.suppressionDefaultsKey
            )
        }
    }

    private func isGranted(_ permission: LaunchPermission) -> Bool {
        PermissionStatusRegistry.provider(for: permission.pane).authorizationState() == .granted
    }
}

/// Launch-time permission prompt. Each row authorizes through
/// PermissionFlow, which opens the matching System Settings pane together
/// with the floating drag-to-authorize panel.
struct PermissionPromptView: View {
    @Bindable var controller: PermissionPromptController
    @State private var doNotRemind = false
    private let didBecomeActive = NotificationCenter.default
        .publisher(for: NSApplication.didBecomeActiveNotification)

    var body: some View {
        VStack(spacing: 20) {
            VStack(spacing: 6) {
                Text("需要以下系统权限")
                    .font(.title3.weight(.semibold))
                Text("授权后 DeskFuse 才能完整提供跨设备的屏幕、键鼠与文件能力，之后可在「设置 → 权限」中随时调整。")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .multilineTextAlignment(.center)
            }
            VStack(spacing: 12) {
                ForEach(controller.presented ?? []) { permission in
                    PermissionPromptRow(permission: permission)
                }
            }
            HStack(spacing: 12) {
                Toggle("不再提醒", isOn: $doNotRemind)
                Spacer()
                Button("稍后") { dismiss() }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    .controlSize(.large)
            }
        }
        .padding(24)
        .frame(width: 480)
        .onReceive(didBecomeActive) { _ in
            controller.reevaluateWhilePresented()
        }
        .onExitCommand { dismiss() }
    }

    private func dismiss() {
        if doNotRemind { controller.suppressPresented() } else { controller.presented = nil }
    }
}

private struct PermissionPromptRow: View {
    let permission: LaunchPermission

    var body: some View {
        HStack(spacing: 16) {
            Image(systemName: permission.symbol)
                .font(.system(size: 20))
                .foregroundStyle(.secondary)
                .frame(width: 28)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 5) {
                Text(permission.title).font(.body.weight(.medium))
                Text(permission.purpose)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            PermissionFlowButton(
                pane: permission.pane,
                suggestedAppURLs: [Bundle.main.bundleURL]
            ) { state in
                Label(state.isGranted ? "已授权" : "前往授权", systemImage: state.systemImage)
                    .foregroundStyle(state.isGranted ? Color.green : Color.primary)
            }
            .buttonStyle(.bordered)
            .frame(width: 104)
        }
        .padding(.vertical, 8)
    }
}
