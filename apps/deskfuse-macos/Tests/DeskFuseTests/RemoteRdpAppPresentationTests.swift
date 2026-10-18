import XCTest
@testable import DeskFuse

final class RemoteRdpAppPresentationTests: XCTestCase {
    func testHashedAliasUsesGenericDisplayName() {
        XCTAssertEqual(RemoteRdpAppPresentation.title(
            name: "deskfuse-7f4a1c", displayName: "chrome", path: #"C:\Program Files\Google\Chrome\Application\chrome.exe"#),
            "Google Chrome")
    }

    func testCustomDisplayNameIsPreserved() {
        XCTAssertEqual(RemoteRdpAppPresentation.title(
            name: "deskfuse-7f4a1c", displayName: "工作浏览器", path: #"C:\Program Files\Google\Chrome\Application\chrome.exe"#),
            "工作浏览器")
    }

    func testWindowsBackslashPathProvidesExecutableName() {
        XCTAssertEqual(RemoteRdpAppPresentation.windowsBasename(#"C:\Windows\System32\notepad.exe"#), "notepad.exe")
        XCTAssertEqual(RemoteRdpAppPresentation.title(
            name: "deskfuse-22a0", displayName: "", path: #"C:\Windows\System32\notepad.exe"#),
            "记事本")
    }

    func testOnlySystemAppsAndExactDiagnosticExecutableAreInternal() {
        XCTAssertTrue(RemoteRdpAppPresentation.isSystemOrInternal(
            displayName: "ShellExperienceHost.exe",
            path: #"C:\Windows\SystemApps\ShellExperienceHost_cw5n1h2txyewy\ShellExperienceHost.exe"#))
        XCTAssertTrue(RemoteRdpAppPresentation.isSystemOrInternal(
            displayName: "IEDIAGCMD.EXE", path: #"C:\Windows\System32\IEDIAGCMD.EXE"#))
        XCTAssertFalse(RemoteRdpAppPresentation.isSystemOrInternal(
            displayName: "记事本", path: #"C:\Windows\System32\notepad.exe"#))
        XCTAssertFalse(RemoteRdpAppPresentation.isSystemOrInternal(
            displayName: "IEDIAG.EXE", path: #"C:\Windows\System32\IEDIAG.EXE"#))
    }
}
