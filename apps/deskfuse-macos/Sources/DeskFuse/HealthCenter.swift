import AppKit
import SwiftUI
import DeskFuseEngine

/// 一条需要用户知道或处理的状态，附带可直接执行的下一步。
struct HealthIssue: Identifiable {
    let id: String
    let tone: DFTone
    /// 侧栏底部只有一项时显示的简短状态。
    let short: String
    let title: String
    var detail: String? = nil
    var actionTitle: String? = nil
    var action: (() -> Void)? = nil
}

/// 从引擎快照、权限与偏好派生全局健康状态；不持有任何业务数据。
@MainActor
enum HealthCenter {
    static func issues(_ store: AppStore) -> [HealthIssue] {
        guard let snapshot = store.snapshot else { return [] }
        var issues: [HealthIssue] = []
        let permissions = store.permissions

        if let error = snapshot.error {
            issues.append(HealthIssue(id: "runtime", tone: .danger, short: "运行异常",
                title: "DeskFuse 后台服务出现问题", detail: friendlyError(error),
                actionTitle: "复制详情") { copy(error) })
        }

        let paired = snapshot.devices.filter { $0.trusted && !$0.blocked }
        if paired.isEmpty {
            issues.append(HealthIssue(id: "no-device", tone: .accent, short: "未连接电脑",
                title: "还没有连接其他电脑",
                detail: "添加另一台电脑后，就能同步剪贴板、共享键鼠和传输文件。",
                actionTitle: "添加电脑") { store.presentAddDevice() })
        } else if !paired.contains(where: \.connected) {
            issues.append(HealthIssue(id: "offline", tone: .warning, short: "设备离线",
                title: "已添加的电脑都不在线",
                detail: "请确认对方已打开 DeskFuse，并和这台 Mac 在同一网络。",
                actionTitle: "刷新") { store.send(.refreshDevices) })
        }

        let input = snapshot.input
        if input.settings.enabled, !permissions.isGranted(.accessibility) {
            issues.append(HealthIssue(id: "input-permission", tone: .warning, short: "需要授权",
                title: "键鼠共享需要「辅助功能」权限",
                detail: AppPermission.accessibility.purpose,
                actionTitle: "前往授权") { permissions.authorize(.accessibility) })
        } else if input.settings.enabled, let error = input.error {
            issues.append(HealthIssue(id: "input", tone: .warning, short: "键鼠暂不可用",
                title: "键鼠共享暂时不可用", detail: friendlyError(error),
                actionTitle: "查看") { store.selectedPage = .input; store.openMain() })
        }

        if #available(macOS 14.2, *), RemoteApplicationManager.shared.enabled,
           !permissions.isGranted(.screenRecording) {
            issues.append(HealthIssue(id: "remote-permission", tone: .warning, short: "需要授权",
                title: "远程应用需要「屏幕录制」权限",
                detail: AppPermission.screenRecording.purpose,
                actionTitle: "前往授权") { permissions.authorize(.screenRecording) })
        }

        if let error = snapshot.clipboardError {
            issues.append(HealthIssue(id: "clipboard", tone: .warning, short: "剪贴板异常",
                title: "剪贴板同步遇到问题", detail: friendlyError(error),
                actionTitle: "查看") { store.selectedPage = .clipboard; store.openMain() })
        }

        let relaunch = permissions.pendingRelaunch
        if !relaunch.isEmpty {
            issues.append(HealthIssue(id: "relaunch", tone: .accent, short: "需要重新打开",
                title: "重新打开 DeskFuse 以启用\(relaunch.map(\.title).joined(separator: "、"))",
                actionTitle: "重新打开") { permissions.relaunch() })
        }

        if let target = snapshot.update.targetVersion {
            issues.append(HealthIssue(id: "update", tone: .accent, short: "有新版本",
                title: "可更新到 \(target)",
                actionTitle: "查看") { store.openSettings(tab: SettingsCategory.about.rawValue) })
        }
        return issues
    }

    /// 侧栏底部的整体色调：取最严重的一项。
    static func tone(of issues: [HealthIssue]) -> DFTone {
        if issues.contains(where: { $0.tone == .danger }) { return .danger }
        if issues.contains(where: { $0.tone == .warning }) { return .warning }
        if issues.isEmpty { return .success }
        return .accent
    }

    static func summary(of issues: [HealthIssue]) -> String {
        switch issues.count {
        case 0: "一切正常"
        case 1: issues[0].short
        default: "\(issues.count) 项需要留意"
        }
    }

    private static func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}

/// 引擎错误多为内部诊断文本。界面先给出可理解的说明，原文只在「复制详情」中保留。
func friendlyError(_ message: String) -> String {
    let lower = message.lowercased()
    let rules: [(keys: [String], text: String)] = [
        (["timed out", "timeout", "超时"], "连接超时，请确认两台电脑在同一网络后重试。"),
        (["connection refused", "unreachable", "no route", "network is down"], "无法连接到对方电脑，请确认对方已打开 DeskFuse。"),
        (["permission", "not permitted", "accessibility", "denied"], "缺少系统权限，请在「设置 → 权限」中授权后重试。"),
        (["disk full", "no space"], "磁盘空间不足，请清理后重试。"),
        (["stream stalled", "transport operation failed"], "网络传输中断，DeskFuse 会自动重试。"),
        (["store operation failed", "database", "sqlite"], "本地数据读写失败，请重新打开 DeskFuse；如果反复出现，请在「设置 → 关于」导出诊断。"),
        (["command channel is closed", "runtime"], "后台服务未就绪，请稍后重试或重新打开 DeskFuse。"),
    ]
    if let rule = rules.first(where: { rule in rule.keys.contains(where: { lower.contains($0) }) }) {
        return rule.text
    }
    // 已是中文的用户提示直接显示；纯英文的内部错误换成通用说明。
    let hasChinese = message.unicodeScalars.contains { (0x4E00...0x9FFF).contains($0.value) }
    return hasChinese ? message : "操作未完成，请稍后重试。如果反复出现，请在「设置 → 关于」导出诊断。"
}
