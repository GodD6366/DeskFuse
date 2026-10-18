import Foundation
import IOKit.pwr_mgt

/// 管理系统休眠抑制，通过 IOKit 的电源管理断言防止系统自动休眠。
@MainActor
final class CaffeineController {
    private var systemAssertionID: IOPMAssertionID = 0
    private var displayAssertionID: IOPMAssertionID = 0
    private(set) var isActive = false

    /// 开启常亮模式，阻止系统自动休眠和显示器休眠
    func enable() {
        guard !isActive else { return }

        let reason = "DeskFuse 保持唤醒" as CFString

        // Keep the machine running and keep the display pipeline awake. These
        // are separate power assertions; the system-sleep assertion alone does
        // not prevent the display from going idle and turning the remote screen
        // black.
        var systemID: IOPMAssertionID = 0
        let systemResult = IOPMAssertionCreateWithName(
            kIOPMAssertionTypePreventUserIdleSystemSleep as CFString,
            IOPMAssertionLevel(kIOPMAssertionLevelOn),
            reason,
            &systemID
        )
        guard systemResult == kIOReturnSuccess else {
            NSLog("DeskFuse: failed to prevent system sleep (IOReturn %d)", systemResult)
            return
        }

        var displayID: IOPMAssertionID = 0
        let displayResult = IOPMAssertionCreateWithName(
            kIOPMAssertionTypePreventUserIdleDisplaySleep as CFString,
            IOPMAssertionLevel(kIOPMAssertionLevelOn),
            reason,
            &displayID
        )
        if displayResult != kIOReturnSuccess {
            // Retain the system assertion even if a platform rejects the
            // display assertion; it still provides the requested sleep guard.
            NSLog("DeskFuse: failed to prevent display sleep (IOReturn %d)", displayResult)
        }

        systemAssertionID = systemID
        displayAssertionID = displayResult == kIOReturnSuccess ? displayID : 0
        isActive = true
    }

    /// 关闭常亮模式，允许系统正常休眠
    func disable() {
        guard isActive else { return }

        if systemAssertionID != 0 { IOPMAssertionRelease(systemAssertionID) }
        if displayAssertionID != 0 { IOPMAssertionRelease(displayAssertionID) }
        systemAssertionID = 0
        displayAssertionID = 0
        isActive = false
    }

    deinit {
        if systemAssertionID != 0 { IOPMAssertionRelease(systemAssertionID) }
        if displayAssertionID != 0 { IOPMAssertionRelease(displayAssertionID) }
    }
}
