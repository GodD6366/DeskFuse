import Foundation
import Observation

/// 只负责展示状态：关闭提示不等于把引擎标记为健康。
/// 可操作的错误一直保留到用户处理或显式关闭，不再数秒后自动消失。
@MainActor @Observable final class ErrorNotice {
    private(set) var message: String?
    /// 原始错误文本；界面只在「复制详情」里暴露它。
    private(set) var rawMessage: String?
    private var localError: String?
    private var runtimeError: String?
    private var dismissedLocalError: String?
    private var dismissedRuntimeError: String?

    func updateLocalError(_ error: String?) {
        localError = error
        // 显式赋值代表一次新的失败，即使文本相同也要重新出现。
        dismissedLocalError = nil
        refresh()
    }

    func updateRuntimeError(_ error: String?) {
        // 重复快照只是观察，不是新的失败：不能复活已被关闭的错误。
        guard runtimeError != error else { return }
        runtimeError = error
        dismissedRuntimeError = nil
        refresh()
    }

    func dismiss() {
        dismissedLocalError = localError
        dismissedRuntimeError = runtimeError
        message = nil
        rawMessage = nil
    }

    private func refresh() {
        let next = (localError != dismissedLocalError ? localError : nil)
            ?? (runtimeError != dismissedRuntimeError ? runtimeError : nil)
        guard next != rawMessage else { return }
        rawMessage = next
        message = next.map(friendlyError)
    }
}
