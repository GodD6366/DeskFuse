import Foundation
import Testing
@testable import DeskFuse

@MainActor @Suite(.serialized) struct OnboardingControllerTests {
    private let key = OnboardingController.completedKey

    @Test func restartClearsCompletionAndPresentsAgain() {
        // 测试进程与应用共用 standard 域里的同名键，结束后恢复原值。
        let original = UserDefaults.standard.object(forKey: key)
        defer { UserDefaults.standard.set(original, forKey: key) }

        let controller = OnboardingController()
        controller.complete()
        #expect(!controller.shouldPresent())
        #expect(!controller.isPresented)

        controller.restart()
        #expect(controller.isPresented)
        // 中途退出后，下次启动仍会继续引导。
        #expect(controller.shouldPresent())
    }
}
