import Foundation
import Observation

/// 全新安装或升级后尚未完成引导的首次启动：先逐项说明并申请系统权限，
/// 再确认默认开启的功能。中途退出后仍会继续，完成后不再重复。
@MainActor @Observable
final class OnboardingController {
    /// 当前引导版本的完成标记。旧版引导使用 v1，不能替代这一版的选择。
    static let completedKey = "deskfuse.onboarding.completed.v2"
    var isPresented = false

    /// 只看本机有没有完成当前引导；与状态库是否存在、应用是否早已安装无关。
    func shouldPresent() -> Bool {
        !UserDefaults.standard.bool(forKey: Self.completedKey)
    }

    func present() { isPresented = true }

    /// 调试用：清除完成标记并立即重走引导，中途退出后下次启动会继续，与全新安装一致。
    func restart() {
        UserDefaults.standard.removeObject(forKey: Self.completedKey)
        isPresented = true
    }

    func complete() {
        UserDefaults.standard.set(true, forKey: Self.completedKey)
        isPresented = false
    }
}
