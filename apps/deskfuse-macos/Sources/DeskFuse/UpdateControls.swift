import SwiftUI
import DeskFuseEngine

extension UpdateState {
    /// 已发现可安装版本或正在下载/待重启，供侧栏快速更新入口使用。
    var showsQuickUpdate: Bool {
        targetVersion != nil || ready || downloading || manualRequested
    }
}

/// 安装包里的更新通道。手动包没有检查服务。
enum UpdateInstallationChannel {
    static var isManual: Bool {
        guard let url = Bundle.main.url(forResource: "update-channel", withExtension: nil),
              let channel = try? String(contentsOf: url, encoding: .utf8) else { return false }
        return channel.trimmingCharacters(in: .whitespacesAndNewlines) == "manual"
    }
}

/// Both entry points use the engine's single updater and its real progress.
struct UpdateControls: View {
    let store: AppStore
    let update: UpdateState

    private var actionTitle: String {
        if update.manualRequested { return update.ready ? "正在立即安装…" : "正在下载安装…" }
        if update.ready { return "立即安装并重启" }
        if update.downloading { return "下载完成后立即安装" }
        return "立即下载安装"
    }

    private var canInstallImmediately: Bool {
        update.ready && update.manualRequested
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            LabeledContent("当前版本", value: "\(update.version) · build \(update.build)")
            LabeledContent("更新来源", value: UpdateInstallationChannel.isManual ? "手动安装" : (update.github ? "GitHub / Homebrew" : "可信局域网"))
            if let target = update.targetVersion {
                LabeledContent("更新版本", value: target)
            }
            Text(update.message).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            if update.downloading && update.totalBytes > 0 {
                ProgressView(value: Double(update.receivedBytes), total: Double(update.totalBytes))
                Text("\(byteCount(update.receivedBytes)) / \(byteCount(update.totalBytes))")
                    .font(.caption).foregroundStyle(.secondary).monospacedDigit()
            }
            if UpdateInstallationChannel.isManual {
                Text("下载新版安装包后手动安装。此版本不会自动检查、下载或安装更新。")
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            } else if update.github {
                ViewThatFits(in: .horizontal) {
                    HStack {
                        installButton
                        if canInstallImmediately { immediateInstallButton }
                        if let address = update.releaseUrl, let url = URL(string: address) {
                            Link("打开 GitHub 发布页", destination: url).buttonStyle(.borderedProminent)
                        }
                        checkButton
                    }
                    VStack(alignment: .leading) {
                        installButton
                        if canInstallImmediately { immediateInstallButton }
                        if let address = update.releaseUrl, let url = URL(string: address) {
                            Link("打开 GitHub 发布页", destination: url).buttonStyle(.borderedProminent)
                        }
                        checkButton
                    }
                }
                Text("GitHub 发布的 Developer ID 签名包会自动下载并安装。已安装的公开发行版也会把本机应用打包，通过可信局域网传给其他 DeskFuse。ad-hoc 包请从发布页下载 DMG 手动安装。")
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            } else {
                ViewThatFits(in: .horizontal) {
                    HStack {
                        installButton
                        if canInstallImmediately { immediateInstallButton }
                        checkButton
                    }
                    VStack(alignment: .leading) {
                        installButton
                        if canInstallImmediately { immediateInstallButton }
                        checkButton
                    }
                }
                Text("从可信局域网设备为这台 Mac 获取已签名新版。立即安装会停止当前传输、粘贴和键鼠共享并重启 DeskFuse。")
                    .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var installButton: some View {
        Button(actionTitle) { store.send(.downloadAndInstallUpdate) }
            .buttonStyle(.borderedProminent)
            .disabled(!update.available || update.manualRequested)
    }

    private var immediateInstallButton: some View {
        Button("立即安装（停止任务）") { store.send(.downloadAndInstallUpdate) }
            .buttonStyle(.borderedProminent)
            .disabled(!canInstallImmediately)
    }

    private var checkButton: some View {
        Button { store.checkForUpdates() } label: {
            HStack(spacing: 6) {
                if store.isCheckingForUpdates { ProgressView().controlSize(.small) }
                Text(store.isCheckingForUpdates ? "正在检查…" : "检查更新")
            }
        }
        .disabled(store.isCheckingForUpdates || update.ready || update.manualRequested)
    }
}
