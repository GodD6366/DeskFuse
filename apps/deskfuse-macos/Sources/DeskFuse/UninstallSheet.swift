import SwiftUI

/// 彻底卸载的两步确认：先勾选要删除的内容（剪贴板历史默认保留），再输入确认词执行。
struct UninstallSheet: View {
    let store: AppStore
    @Environment(\.dismiss) private var dismiss
    @State private var step: Step = .review
    @State private var confirmation = ""
    @State private var running = false
    @State private var failure: String?
    @State private var plan = DeskFuseUninstaller.Plan.current()
    @State private var selection = DeskFuseUninstaller.Plan.current().defaultSelection

    static let confirmationWord = "卸载"

    private enum Step { case review, confirm }

    var body: some View {
        VStack(alignment: .leading, spacing: DFSpace.l) {
            HStack(alignment: .top, spacing: DFSpace.m) {
                Image(systemName: "trash")
                    .font(DFType.icon(.large))
                    .foregroundStyle(DFPalette.danger)
                    .frame(width: 32)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: DFSpace.xs) {
                    Text("彻底卸载 DeskFuse").font(DFType.pageTitle)
                    Text(step == .review ? "勾选要从这台 Mac 上永久删除的内容，删除后无法恢复。"
                                         : "最后确认：卸载完成后 DeskFuse 会立即退出。")
                        .font(DFType.body)
                        .dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
            }

            switch step {
            case .review: reviewList
            case .confirm: confirmField
            }

            if let failure {
                DFNotice(tone: .danger, title: "卸载未完成", message: failure)
            }

            HStack(spacing: DFSpace.m) {
                Spacer(minLength: 0)
                Button("取消") { dismiss() }
                    .buttonStyle(.dfSecondary)
                    .keyboardShortcut(.cancelAction)
                    .disabled(running)
                switch step {
                case .review:
                    Button("继续…") { withMotion(DFMotion.snappy) { step = .confirm } }
                        .buttonStyle(.dfSecondary)
                        .disabled(selection.isEmpty)
                case .confirm:
                    Button(running ? "正在卸载…" : "彻底卸载并退出", role: .destructive, action: uninstall)
                        .buttonStyle(.dfPrimary)
                        .tint(DFPalette.danger)
                        .disabled(running || confirmation.trimmingCharacters(in: .whitespaces) != Self.confirmationWord)
                }
            }
        }
        .padding(DFSpace.xl)
        .frame(width: 500)
        .interactiveDismissDisabled(running)
    }

    private var reviewList: some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            ForEach(plan.availableComponents) { component in
                Toggle(isOn: binding(for: component)) {
                    VStack(alignment: .leading, spacing: DFSpace.xxs) {
                        Text(title(for: component)).font(DFType.body)
                        Text(detail(for: component)).font(DFType.detail).dfSecondaryText()
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .toggleStyle(.checkbox)
            }
            VStack(alignment: .leading, spacing: DFSpace.xs) {
                Text("其他已配对设备上仍会保留这台 Mac 的记录，可在对方的设备页中移除。")
                if plan.installedByHomebrew, selection.contains(.application) {
                    Text("检测到通过 Homebrew 安装，卸载后请在终端执行 `brew uninstall --cask deskfuse` 清理安装记录。")
                }
            }
            .font(DFType.detail)
            .dfSecondaryText()
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(DFMetrics.cardPadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .dfCard()
    }

    private func binding(for component: DeskFuseUninstaller.Component) -> Binding<Bool> {
        Binding(get: { selection.contains(component) },
                set: { on in
                    if on { selection.insert(component) } else { selection.remove(component) }
                })
    }

    private func title(for component: DeskFuseUninstaller.Component) -> String {
        switch component {
        case .userData: "剪贴板历史与本机数据"
        case .caches: "缓存、日志与更新下载"
        case .preferences: "偏好设置"
        case .keychain: "钥匙串中的 Windows 远程应用账号"
        case .loginItem: "开机自动启动项"
        case .privacy: "系统权限记录"
        case .microphoneDriver: "麦克风驱动"
        case .application: "DeskFuse 应用本身"
        }
    }

    private func detail(for component: DeskFuseUninstaller.Component) -> String {
        switch component {
        case .userData:
            "剪贴板历史、导入的文件、配对记录与设备身份（~/Library/Application Support/DeskFuse）。"
                + "三者存放在同一数据库中，只能一起保留或删除；保留后重装可继续使用。"
        case .caches: "可重新生成，不含剪贴板历史。"
        case .preferences: "界面与功能开关等设置。"
        case .keychain: "重装后需要重新输入 Windows 账号密码。"
        case .loginItem: "登录 macOS 时不再自动打开 DeskFuse。"
        case .privacy: "辅助功能、屏幕录制、输入监控等，重装后需重新授权。"
        case .microphoneDriver: "需要输入管理员密码，并会重启音频服务。"
        case .application: "移到废纸篓。"
        }
    }

    private var confirmField: some View {
        VStack(alignment: .leading, spacing: DFSpace.s) {
            DFNotice(tone: selection.contains(.userData) ? .danger : .accent,
                     title: selection.contains(.userData) ? "剪贴板历史将被永久删除" : "剪贴板历史将保留",
                     message: "将删除：" + plan.availableComponents.filter(selection.contains)
                        .map(title(for:)).joined(separator: "、") + "。")
            Text("请输入“\(Self.confirmationWord)”以确认：")
                .font(DFType.body)
            TextField(Self.confirmationWord, text: $confirmation)
                .textFieldStyle(.roundedBorder)
                .disabled(running)
                .onSubmit { if confirmation == Self.confirmationWord { uninstall() } }
        }
    }

    private func uninstall() {
        guard !running else { return }
        running = true
        failure = nil
        Task { @MainActor in
            do {
                let completed = try await DeskFuseUninstaller.perform(store: store, plan: plan, selection: selection)
                // 成功时应用已经开始退出；取消管理员授权则什么都没删除。
                if !completed {
                    running = false
                    failure = "已取消管理员授权，没有删除任何内容。"
                }
            } catch {
                running = false
                failure = error.localizedDescription
                plan = .current()
                selection.formIntersection(plan.availableComponents)
            }
        }
    }
}
