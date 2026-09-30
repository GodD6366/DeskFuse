import Foundation
import IOKit.pwr_mgt

/// 管理系统休眠抑制，通过 IOKit 的电源管理断言防止系统自动休眠。
@MainActor
final class CaffeineController {
    private var assertionID: IOPMAssertionID = 0
    private(set) var isActive = false

    /// 开启常亮模式，阻止系统自动休眠和显示器休眠
    func enable() {
        guard !isActive else { return }

        var id: IOPMAssertionID = 0
        let reason = "DeskFuse 保持唤醒" as CFString

        // kIOPMAssertionTypePreventUserIdleSystemSleep: 阻止用户闲置导致的系统休眠
        // kIOPMAssertionTypePreventUserIdleDisplaySleep: 阻止显示器休眠
        let assertionType = kIOPMAssertionTypePreventUserIdleSystemSleep as CFString

        let result = IOPMAssertionCreateWithName(
            assertionType,
            IOPMAssertionLevel(kIOPMAssertionLevelOn),
            reason,
            &id
        )

        if result == kIOReturnSuccess {
            assertionID = id
            isActive = true
        }
    }

    /// 关闭常亮模式，允许系统正常休眠
    func disable() {
        guard isActive else { return }

        IOPMAssertionRelease(assertionID)
        assertionID = 0
        isActive = false
    }

    deinit {
        if isActive {
            IOPMAssertionRelease(assertionID)
        }
    }
}
