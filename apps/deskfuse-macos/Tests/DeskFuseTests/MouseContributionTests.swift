import XCTest
@testable import DeskFuse

final class MouseContributionTests: XCTestCase {
    func testMarkdownIncludesIdentityAndButtonsInNumericOrder() {
        let draft = MouseContributionDraft(
            modelName: "MX Master 4",
            connection: .bluetooth,
            vendorID: "046d",
            productID: "0xB042",
            detectedName: "MX Master 4",
            isSupported: false,
            notes: "侧键默认发送 button 3|4",
            samples: [
                MouseContributionSample(number: 5, name: "拇指键", count: 2),
                MouseContributionSample(number: 3, name: "后退", count: 1),
            ],
            appVersion: "0.6.1",
            appBuild: "123",
            systemVersion: "macOS 15.0",
            capturedAt: Date(timeIntervalSince1970: 0))
        let markdown = draft.markdown()
        XCTAssertTrue(markdown.contains("# 鼠标适配数据：MX Master 4"))
        XCTAssertTrue(markdown.contains("- Vendor ID：0x046D"))
        XCTAssertTrue(markdown.contains("- Product ID：0xB042"))
        XCTAssertTrue(markdown.contains("- 当前适配状态：未适配（通用鼠标）"))
        XCTAssertTrue(markdown.contains("侧键默认发送 button 3|4"))
        XCTAssertTrue(markdown.contains("| 3 | 后退 | 1 |"))
        XCTAssertTrue(markdown.contains("| 5 | 拇指键 | 2 |"))
        XCTAssertLessThan(
            markdown.range(of: "| 3 | 后退 |")!.lowerBound,
            markdown.range(of: "| 5 | 拇指键 |")!.lowerBound)
    }

    func testMarkdownWithoutSamplesExplainsEmptyCapture() {
        let draft = MouseContributionDraft(
            modelName: "",
            connection: .receiver,
            vendorID: "",
            productID: "",
            detectedName: nil,
            isSupported: false,
            notes: "",
            samples: [],
            appVersion: "0.6.1",
            appBuild: "",
            systemVersion: "macOS 15.0",
            capturedAt: Date(timeIntervalSince1970: 0))
        let markdown = draft.markdown()
        XCTAssertTrue(markdown.contains("# 鼠标适配数据：未命名鼠标"))
        XCTAssertTrue(markdown.contains("- Vendor ID：未提供"))
        XCTAssertTrue(markdown.contains("- 系统检测名称：未提供"))
        XCTAssertTrue(markdown.contains("未记录到按键。"))
    }

    func testTransportLabelsMapToConnectionChoice() {
        XCTAssertEqual(MouseConnection(transport: "BluetoothLowEnergy"), .bluetooth)
        XCTAssertEqual(MouseConnection(transport: "USB"), .receiver)
        XCTAssertEqual(MouseConnection(transport: "SPU"), .wired)
        XCTAssertEqual(MouseConnection(transport: ""), .wired)
    }

    func testSupportedModelNamesCoverThreeSAndRejectOtherG502Variants() {
        XCTAssertEqual(LogitechHIDProtocol.model("MX Master 3S")?.displayName, "MX Master 3 / 3S")
        XCTAssertEqual(LogitechHIDProtocol.model("G502 LIGHTSPEED Wireless Gaming Mouse")?.displayName,
                       "G502 LIGHTSPEED")
        XCTAssertNil(LogitechHIDProtocol.model("G502 X LIGHTSPEED"))
        XCTAssertNil(LogitechHIDProtocol.model("USB Receiver"))
    }
}
