import SwiftUI

/// 虚拟麦克风节点内的设置入口；操作仍由页面交给现有驱动管理流程。
struct MicrophoneDriverControls: View {
    let installed: Bool
    let loaded: Bool
    let legacyInstalled: Bool
    let defaultInput: Bool
    let operationTitle: String?
    let install: () -> Void
    let restart: () -> Void
    let setDefault: () -> Void

    var body: some View {
        VStack(spacing: DFSpace.s) {
            if let operationTitle {
                ProgressView().controlSize(.small)
                Text(operationTitle)
                    .font(DFType.caption).dfSecondaryText()
                Text("请完成系统管理员授权")
                    .font(DFType.caption).dfSecondaryText()
            } else if !installed || legacyInstalled {
                status(legacyInstalled ? "需要更新驱动" : "驱动未安装",
                       symbol: "arrow.down.circle", tone: .accent)
                Button(legacyInstalled ? "更新驱动…" : "安装驱动…", action: install)
                    .buttonStyle(.dfPrimary)
                    .help(legacyInstalled ? "安装新驱动并移除旧版驱动" : "安装虚拟麦克风，需要管理员授权")
            } else if !loaded {
                status("驱动未就绪", symbol: "exclamationmark.circle.fill", tone: .warning)
                Button("重启音频服务…", action: restart)
                    .buttonStyle(.dfPrimary)
                    .help("短暂中断此 Mac 的录音和播放，恢复后重新检测驱动")
            } else if !defaultInput {
                status("默认输入待设置", symbol: "exclamationmark.circle.fill", tone: .warning)
                Button("设为默认输入", action: setDefault)
                    .buttonStyle(.dfPrimary)
                    .help("将 DeskFuse 麦克风设为系统默认输入")
            }
        }
        .multilineTextAlignment(.center)
        .fixedSize(horizontal: false, vertical: true)
    }

    private func status(_ title: String, symbol: String, tone: DFTone) -> some View {
        Label(title, systemImage: symbol)
            .font(DFType.caption)
            .foregroundStyle(tone.color)
    }
}
