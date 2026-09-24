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
    case wifi

    var id: String { rawValue }

    var pane: PermissionFlowPane? {
        switch self {
        case .accessibility: .accessibility
        case .fullDiskAccess: .fullDiskAccess
        case .screenRecording: .screenRecording
        // Location authorization has no PermissionFlow pane; the row requests
        // it through the CoreLocation entry point instead.
        case .wifi: nil
        }
    }

    var title: String {
        switch self {
        case .accessibility: "辅助功能"
        case .fullDiskAccess: "完全磁盘访问"
        case .screenRecording: "屏幕录制"
        case .wifi: "Wi-Fi 名称（定位）"
        }
    }

    var purpose: String {
        switch self {
        case .accessibility: "拦截文件粘贴、检查 Finder 焦点与控制键鼠"
        case .fullDiskAccess: "访问受系统保护的文件与目录"
        case .screenRecording: "远程应用访问本机窗口时捕获屏幕内容"
        case .wifi: "读取当前 Wi-Fi 名称，用于按网络保存主控与布局；不采集位置"
        }
    }

    var symbol: String {
        switch self {
        case .accessibility: "accessibility"
        case .fullDiskAccess: "folder.badge.gearshape"
        case .screenRecording: "rectangle.dashed.badge.record"
        case .wifi: "wifi"
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
    var hasOpenedAuthorization = false

    static let suppressionDefaultsKey = "deskfuse.permissionPrompt.suppressed"

    private var evaluatedThisLaunch = false
    private let authorizationFlow = PermissionFlowController()
    private var authorizationTask: Task<Void, Never>?
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

    func authorize(_ permission: LaunchPermission) {
        hasOpenedAuthorization = true
        authorizationTask?.cancel()
        if permission == .wifi {
            WifiPermission.request()
            authorizationTask = Task { @MainActor [weak self] in
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .seconds(1)) } catch { return }
                    guard let self, self.isGranted(.wifi) else { continue }
                    self.reevaluateWhilePresented()
                    AppStore.shared.send(.refreshPermissions)
                    AppStore.shared.openMain()
                    return
                }
            }
            return
        }
        guard let pane = permission.pane else { return }
        authorizationFlow.setLocaleIdentifier("zh-Hans")
        authorizationFlow.authorize(pane: pane, suggestedAppURLs: [Bundle.main.bundleURL])
        authorizationTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                guard let self else { return }
                guard self.isGranted(permission) else { continue }
                self.authorizationFlow.closePanel()
                self.reevaluateWhilePresented()
                AppStore.shared.send(.refreshPermissions)
                AppStore.shared.openMain()
                return
            }
        }
    }

    func restartAfterAuthorization() {
        let launcher = Process()
        launcher.executableURL = URL(fileURLWithPath: "/bin/sh")
        // Relaunch only after this process exits and releases its instance lock.
        // Positional arguments keep application paths out of shell source.
        launcher.arguments = ["-c", """
            for attempt in $(/usr/bin/seq 1 150); do
                if ! kill -0 "$1" 2>/dev/null; then
                    exec /usr/bin/open -n "$2"
                fi
                /bin/sleep 0.2
            done
            exit 1
            """, "deskfuse-relaunch", String(ProcessInfo.processInfo.processIdentifier),
            Bundle.main.bundleURL.path]
        do {
            try launcher.run()
            authorizationTask?.cancel()
            authorizationFlow.closePanel()
            NSApp.terminate(nil)
        } catch {
            AppStore.shared.error = "无法重启应用：\(error.localizedDescription)"
        }
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
        if permission == .wifi { return WifiPermission.isGranted }
        guard let pane = permission.pane else { return false }
        return PermissionStatusRegistry.provider(for: pane).authorizationState() == .granted
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
                    PermissionPromptRow(permission: permission, controller: controller)
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
            if controller.hasOpenedAuthorization {
                Button("重启以完成授权", systemImage: "arrow.clockwise") {
                    controller.restartAfterAuthorization()
                }
            }
        }
        .padding(24)
        .frame(width: 480)
        .background(SystemRestartAllowance())
        .onReceive(didBecomeActive) { _ in
            controller.reevaluateWhilePresented()
        }
        .onExitCommand { dismiss() }
    }

    private func dismiss() {
        if doNotRemind { controller.suppressPresented() } else { controller.presented = nil }
    }
}

/// 系统设置在授予屏幕录制等权限后，会向本进程发送退出并重新打开。
/// SwiftUI sheet 默认在模态期间拒绝终止，那个重启按钮就会没有效果。
private struct SystemRestartAllowance: NSViewRepresentable {
    func makeNSView(context: Context) -> SystemRestartAllowanceView { SystemRestartAllowanceView() }
    func updateNSView(_ view: SystemRestartAllowanceView, context: Context) { view.allowSystemRestart() }
}

@MainActor
private final class SystemRestartAllowanceView: NSView {
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        allowSystemRestart()
    }

    override func layout() {
        super.layout()
        allowSystemRestart()
    }

    /// 授权内容有时不在 sheet 窗口里，而在它的父窗口中。
    /// 弹窗还在时，放开当前这张 sheet，系统设置才能把应用退出并重新打开。
    func allowSystemRestart() {
        if let modal = NSApp.modalWindow {
            allowTermination(of: modal)
        }
        for window in NSApp.windows {
            if let sheet = window.attachedSheet {
                allowTermination(of: sheet)
            }
            if window.isSheet {
                allowTermination(of: window)
            }
        }
    }

    private func allowTermination(of window: NSWindow) {
        guard window.preventsApplicationTerminationWhenModal else { return }
        window.preventsApplicationTerminationWhenModal = false
    }
}

private struct PermissionPromptRow: View {
    let permission: LaunchPermission
    let controller: PermissionPromptController

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
            Button {
                controller.authorize(permission)
            } label: {
                Label("前往授权", systemImage: "arrow.up.forward.app")
            }
            .buttonStyle(.bordered)
            .frame(width: 104)
        }
        .padding(.vertical, 8)
    }
}
