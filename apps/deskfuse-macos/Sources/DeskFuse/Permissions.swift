import AppKit
import IOKit.hid
import Observation
import SwiftUI
import PermissionFlow
import PermissionFlowScreenRecordingStatus

/// DeskFuse 用到的系统权限。全部按需申请：只有对应功能被开启或被权限阻断时才提示。
enum AppPermission: String, CaseIterable, Identifiable {
    case accessibility
    case screenRecording
    case inputMonitoring
    case wifi
    case fullDiskAccess

    var id: String { rawValue }

    var title: String {
        switch self {
        case .accessibility: "辅助功能"
        case .screenRecording: "屏幕录制"
        case .inputMonitoring: "输入监控"
        case .wifi: "Wi-Fi 名称（定位）"
        case .fullDiskAccess: "完全磁盘访问"
        }
    }

    /// 用普通用户能理解的话说明为什么需要。
    var purpose: String {
        switch self {
        case .accessibility: "让键鼠在电脑间切换，并把剪贴板内容粘贴回原来的应用。"
        case .screenRecording: "远程应用需要它来显示另一台 Mac 上的窗口。"
        case .inputMonitoring: "鼠标按键功能需要它来识别鼠标侧键。"
        case .wifi: "按 Wi-Fi 自动切换主控电脑时读取网络名称；不会获取位置。"
        case .fullDiskAccess: "可选。仅在传输受系统保护的文件夹时需要。"
        }
    }

    var symbol: String {
        switch self {
        case .accessibility: "accessibility"
        case .screenRecording: "rectangle.dashed.badge.record"
        case .inputMonitoring: "computermouse"
        case .wifi: "wifi"
        case .fullDiskAccess: "folder.badge.gearshape"
        }
    }

    fileprivate var pane: PermissionFlowPane? {
        switch self {
        case .accessibility: .accessibility
        case .screenRecording: .screenRecording
        case .fullDiskAccess: .fullDiskAccess
        case .inputMonitoring, .wifi: nil
        }
    }

    /// macOS 要求重新打开应用后，这些授权才会对当前进程生效。
    var requiresRelaunch: Bool { self == .screenRecording || self == .fullDiskAccess }

    fileprivate var settingsURL: URL? {
        let anchor = switch self {
        case .accessibility: "Privacy_Accessibility"
        case .screenRecording: "Privacy_ScreenCapture"
        case .inputMonitoring: "Privacy_ListenEvent"
        case .wifi: "Privacy_LocationServices"
        case .fullDiskAccess: "Privacy_AllFiles"
        }
        return URL(string: "x-apple.systempreferences:com.apple.preference.security?\(anchor)")
    }
}

/// 兼容旧页面的名称；权限实现已统一由 PermissionCenter 管理。
typealias LaunchPermission = AppPermission

/// 权限状态的唯一来源。读取只走公开的查询接口，不会触发系统弹窗；
/// 应用重新激活或授权轮询时递增 `revision`，让界面重新读取状态。
@MainActor
@Observable
final class PermissionCenter {
    private(set) var revision = 0
    private let authorizationFlow = PermissionFlowController()
    private let wifiPermission = WifiPermissionAccess.shared
    @ObservationIgnored private var authorizationTask: Task<Void, Never>?
    @ObservationIgnored private var activationObserver: NSObjectProtocol?
    /// 启动时已授权的权限；之后才授予、且需要重新打开才生效的会提示重新打开。
    @ObservationIgnored private var grantedAtLaunch: Set<AppPermission> = []

    init() {
        PermissionFlowScreenRecordingStatus.register()
        grantedAtLaunch = Set(AppPermission.allCases.filter(probe))
        activationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
    }

    func refresh() {
        wifiPermission.refresh()
        revision &+= 1
    }

    func isGranted(_ permission: AppPermission) -> Bool {
        _ = revision
        return probe(permission)
    }

    /// 已在本次运行中授予、但需要重新打开 DeskFuse 才生效的权限。
    var pendingRelaunch: [AppPermission] {
        AppPermission.allCases.filter {
            $0.requiresRelaunch && !grantedAtLaunch.contains($0) && isGranted($0)
        }
    }

    func authorize(_ permission: AppPermission) {
        authorizationTask?.cancel()
        authorizationFlow.closePanel()
        if isGranted(permission) {
            openSettings(permission)
            return
        }
        switch permission {
        case .wifi:
            wifiPermission.authorize()
        case .inputMonitoring:
            if !IOHIDRequestAccess(kIOHIDRequestTypeListenEvent) { openSettings(permission) }
        default:
            guard let pane = permission.pane else { return }
            authorizationFlow.setLocaleIdentifier("zh-Hans")
            authorizationFlow.authorize(pane: pane, suggestedAppURLs: [Bundle.main.bundleURL])
        }
        // 用户在系统设置里授权后自动收起引导浮板，并刷新引擎侧状态。
        authorizationTask = Task { @MainActor [weak self] in
            for _ in 0..<300 {
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
                guard let self else { return }
                guard self.probe(permission) else { continue }
                self.authorizationFlow.closePanel()
                self.refresh()
                AppStore.shared.send(.refreshPermissions)
                AppStore.shared.openMain()
                return
            }
        }
    }

    func openSettings(_ permission: AppPermission) {
        if permission == .wifi { wifiPermission.openSettings(); return }
        if let url = permission.settingsURL { NSWorkspace.shared.open(url) }
    }

    /// 等当前进程退出并释放单实例锁后，再由系统重新打开 DeskFuse。
    func relaunch() {
        let launcher = Process()
        launcher.executableURL = URL(fileURLWithPath: "/bin/sh")
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
            AppStore.shared.error = "无法重新打开 DeskFuse，请手动退出后再打开。"
        }
    }

    private func probe(_ permission: AppPermission) -> Bool {
        switch permission {
        case .wifi:
            return wifiPermission.state == .granted
        case .inputMonitoring:
            return IOHIDCheckAccess(kIOHIDRequestTypeListenEvent) == kIOHIDAccessTypeGranted
        default:
            guard let pane = permission.pane else { return false }
            return PermissionStatusRegistry.provider(for: pane).authorizationState() == .granted
        }
    }
}

/// 权限行：用途说明 + 当前状态 + 授权按钮。在功能页、引导和设置里共用。
struct PermissionRow: View {
    let permission: AppPermission
    var compact = false
    private var center: PermissionCenter { AppStore.shared.permissions }

    var body: some View {
        let granted = center.isGranted(permission)
        HStack(spacing: DFSpace.m) {
            Image(systemName: permission.symbol)
                .font(DFType.icon(.medium))
                .dfSecondaryText()
                .frame(width: 24)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: DFSpace.xxs) {
                Text(permission.title).font(DFType.bodyMedium)
                if !compact {
                    Text(permission.purpose)
                        .font(DFType.detail)
                        .dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if granted {
                Label("已授权", systemImage: "checkmark.circle.fill")
                    .font(DFType.detail)
                    .foregroundStyle(DFPalette.success)
                Button("打开设置") { center.openSettings(permission) }
                    .buttonStyle(.dfPlain)
            } else {
                Button("前往授权") { center.authorize(permission) }
                    .buttonStyle(.dfPrimary)
            }
        }
        .padding(.vertical, DFSpace.xs)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(permission.title)，\(granted ? "已授权" : "未授权")")
    }
}

/// 授权需要重新打开应用时的提示；仅在确有需要时出现。
struct PermissionRelaunchNotice: View {
    private var center: PermissionCenter { AppStore.shared.permissions }

    var body: some View {
        let pending = center.pendingRelaunch
        if !pending.isEmpty {
            DFNotice(tone: .accent,
                     title: "重新打开 DeskFuse 以启用\(pending.map(\.title).joined(separator: "、"))",
                     message: "macOS 需要重新打开应用后，新授予的权限才会生效。",
                     actionTitle: "重新打开") { center.relaunch() }
        }
    }
}

/// 系统设置在授予屏幕录制等权限后，会向本进程发送退出并重新打开。
/// SwiftUI sheet 默认在模态期间拒绝终止，那个重启按钮就会没有效果。
struct SystemRestartAllowance: NSViewRepresentable {
    func makeNSView(context: Context) -> SystemRestartAllowanceView { SystemRestartAllowanceView() }
    func updateNSView(_ view: SystemRestartAllowanceView, context: Context) { view.allowSystemRestart() }
}

@MainActor
final class SystemRestartAllowanceView: NSView {
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

extension View {
    /// 弹窗显示期间允许系统设置「退出并重新打开」本应用。
    func allowsSystemRelaunch() -> some View { background(SystemRestartAllowance()) }
}
