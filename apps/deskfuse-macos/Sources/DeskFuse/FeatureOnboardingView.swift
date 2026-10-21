import SwiftUI

/// 功能总开关关闭时唯一的引导页：演示动画 + 说明 + 启用按钮。
/// 启用按钮与右上角总开关是同一个动作，页面不再出现第二种“已关闭”状态。
struct FeatureOnboardingView: View {
    let store: AppStore
    let page: Page

    var body: some View {
        if let content = FeatureOnboardingContent(page: page) {
            FeatureGateView(
                symbol: page.symbol,
                title: page.rawValue,
                message: content.message,
                bullets: content.bullets,
                permission: content.permission,
                actionTitle: "启用\(page.rawValue)",
                action: { store.setFeatureEnabled(true, for: page) {} },
                secondaryTitle: content.opensSettings ? "打开设置" : nil,
                secondaryAction: content.opensSettings
                    ? { store.openSettings(tab: SettingsCategory.general.rawValue) } : nil,
                illustration: content.illustration
            )
            .padding(28)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

/// 各功能引导页的文案、所需权限与演示动画。
struct FeatureOnboardingContent {
    let message: String
    let bullets: [String]
    let permission: LaunchPermission?
    var opensSettings = false
    let illustration: AnyView

    init?(page: Page) {
        switch page {
        case .input:
            // 只打开总开关，不替用户把本机设为主控：主控按“最后设为主控者生效”仲裁，
            // 在被控设备上误设主控会抢走另一台的控制权。
            message = "用一套键鼠控制多台电脑：把鼠标移出屏幕边缘，就进入相邻的设备。"
            bullets = [
                "鼠标穿越屏幕边缘切换设备，键盘随之跟随",
                "开启后此 Mac 可被主控控制；在键鼠所在的电脑上设为主控",
            ]
            permission = .accessibility
            illustration = AnyView(InputCrossingDemo())
        case .clipboard:
            message = "复制过的内容都会留下，并在已配对的电脑之间同步。"
            bullets = [
                "在一台电脑复制，另一台直接粘贴",
                "历史记录随时搜索、再次使用",
            ]
            permission = nil
            illustration = AnyView(ClipboardSyncDemo())
        case .files:
            message = "把文件直接发给同一局域网里的电脑，不经过云端。"
            bullets = [
                "拖入文件，选择在线的电脑即可发送",
                "手机扫码也能发送照片或文字",
            ]
            permission = nil
            illustration = AnyView(FileTransferDemo())
        case .remoteApplications:
            message = "在这台 Mac 上打开另一台电脑的应用窗口。"
            bullets = [
                "应用窗口像本地窗口一样显示与操作",
                "支持 macOS 与 Windows 远端",
            ]
            permission = .screenRecording
            opensSettings = true
            illustration = AnyView(RemoteWindowDemo())
        case .microphone:
            message = "把另一台 Mac 的麦克风当作这台 Mac 的输入设备。"
            bullets = [
                "应用读取麦克风时才传输，音频不会保存",
                "两台 Mac 需先在“设备”中完成配对",
            ]
            permission = nil
            illustration = AnyView(MicrophoneRelayDemo())
        case .mouse:
            message = "给鼠标侧键分配常用动作。"
            bullets = [
                "按一下侧键即可切换桌面等系统动作",
                "未适配的鼠标型号自动使用通用映射",
            ]
            permission = .inputMonitoring
            illustration = AnyView(MouseSideButtonDemo())
        case .devices, .settings:
            return nil
        }
    }
}
