import SwiftUI
import DeskFuseEngine

extension UpdateState {
    /// 已发现可安装版本或正在下载/待重启，供侧栏快速更新入口使用。
    var showsQuickUpdate: Bool {
        targetVersion != nil || ready || downloading || manualRequested
    }
}

/// 安装包里的更新通道（由打包脚本写入 `update-channel`）。开发构建没有该资源。
enum UpdateInstallationChannel: String {
    /// GitHub Release / Homebrew 公开发行版。
    case github
    /// 手动安装包：不检查、不下载更新。
    case manual
    /// 可信局域网分发的内部版本。
    case lan
    /// 本地开发构建。
    case development

    static let current: Self = {
        guard let url = Bundle.main.url(forResource: "update-channel", withExtension: nil),
              let text = try? String(contentsOf: url, encoding: .utf8) else { return .development }
        return Self(rawValue: text.trimmingCharacters(in: .whitespacesAndNewlines)) ?? .development
    }()

    static var isManual: Bool { current == .manual }

    /// 内部通道才向用户展示 build 号、批量触发对端更新等维护操作。
    var isInternal: Bool { self == .lan || self == .development }

    var title: String {
        switch self {
        case .github: "GitHub 发行版"
        case .manual: "手动安装"
        case .lan: "局域网内部版"
        case .development: "开发版"
        }
    }
}

/// Both entry points use the engine's single updater and its real progress.
struct UpdateControls: View {
    let store: AppStore
    let update: UpdateState

    private var actionTitle: String {
        if update.manualRequested { return update.ready ? "正在安装…" : "正在下载…" }
        if update.ready { return "安装并重新打开" }
        if update.downloading { return "下载完成后安装" }
        return "下载并安装"
    }

    private var statusText: String {
        if UpdateInstallationChannel.isManual { return "此版本需要手动下载新版安装包。" }
        if let target = update.targetVersion { return "发现新版本 \(target)。" }
        if update.checking || store.isCheckingForUpdates { return "正在检查更新…" }
        return update.message.isEmpty ? "已是最新版本。" : update.message
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DFSpace.m) {
            HStack(alignment: .firstTextBaseline, spacing: DFSpace.m) {
                VStack(alignment: .leading, spacing: DFSpace.xxs) {
                    Text("DeskFuse \(update.version)").font(DFType.bodyMedium)
                    Text(statusText)
                        .font(DFType.detail)
                        .dfSecondaryText()
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if !UpdateInstallationChannel.isManual {
                    if update.targetVersion != nil || update.ready || update.downloading {
                        Button(actionTitle) { store.send(.downloadAndInstallUpdate) }
                            .buttonStyle(.dfPrimary)
                            .disabled(!update.available || update.manualRequested)
                    }
                    checkButton
                }
            }
            if update.downloading && update.totalBytes > 0 {
                ProgressView(value: Double(update.receivedBytes), total: Double(update.totalBytes))
                Text("\(byteCount(update.receivedBytes)) / \(byteCount(update.totalBytes))")
                    .font(DFType.detail).dfSecondaryText().monospacedDigit()
            }
            if update.ready {
                Text("安装时会暂停正在进行的传输和键鼠共享，并自动重新打开 DeskFuse。")
                    .font(DFType.detail).dfSecondaryText()
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let address = update.releaseUrl, let url = URL(string: address) {
                Link("查看更新说明", destination: url).font(DFType.detail)
            }
        }
    }

    private var checkButton: some View {
        Button { store.checkForUpdates() } label: {
            HStack(spacing: DFSpace.xs) {
                if store.isCheckingForUpdates { ProgressView().controlSize(.small) }
                Text(store.isCheckingForUpdates ? "正在检查…" : "检查更新")
            }
        }
        .buttonStyle(.dfSecondary)
        .disabled(store.isCheckingForUpdates || update.ready || update.manualRequested)
    }
}
