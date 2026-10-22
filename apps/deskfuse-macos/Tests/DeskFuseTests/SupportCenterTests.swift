import XCTest
@testable import DeskFuse

@MainActor final class SupportCenterTests: XCTestCase {
    private let environment = [("DeskFuse", "v1.0.0（GitHub 发行版）"), ("当前页面", "剪贴板")]

    private func query(_ url: URL) -> [String: String] {
        let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        return Dictionary(uniqueKeysWithValues: items.map { ($0.name, $0.value ?? "") })
    }

    func testRepositoryMatchesDistributionConfig() throws {
        // Tests/DeskFuseTests/ → 仓库根目录的 config/distribution.json
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let data = try Data(contentsOf: root.appendingPathComponent("config/distribution.json"))
        let config = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(config["source_repository"] as? String, SupportCenter.repository)
        XCTAssertEqual(config["release_repository"] as? String, SupportCenter.repository)
    }

    func testIssueLinkTargetsSourceRepository() {
        let url = SupportCenter.newIssueURL(environment: environment, error: nil, summary: nil, diagnostics: nil)
        XCTAssertEqual(url.host, "github.com")
        XCTAssertEqual(url.path, "/GodD6366/DeskFuse/issues/new")
        let body = query(url)["body"] ?? ""
        XCTAssertTrue(body.contains("- DeskFuse：v1.0.0（GitHub 发行版）"))
        XCTAssertTrue(body.contains("未能导出完整诊断"))
        XCTAssertFalse(body.contains("## 界面提示"))
        XCTAssertFalse(body.contains("## 诊断摘要"))
    }

    func testIssueLinkCarriesFriendlyTitleSummaryAndRedactedPaths() {
        let raw = "transport operation failed at \(NSHomeDirectory())/Library/x"
        let diagnostics = URL(fileURLWithPath: NSHomeDirectory() + "/Library/Logs/DeskFuse/d.jsonl")
        let url = SupportCenter.newIssueURL(environment: environment, error: raw,
                                            summary: "version=1.0.0\nrecords: runtime=3\n2 Error runtime/connect",
                                            diagnostics: diagnostics)
        let fields = query(url)
        XCTAssertEqual(fields["title"], "[macOS] 网络传输中断，DeskFuse 会自动重试。")
        let body = fields["body"] ?? ""
        XCTAssertTrue(body.contains("transport operation failed at ~/Library/x"))
        XCTAssertTrue(body.contains("`~/Library/Logs/DeskFuse/d.jsonl`"))
        XCTAssertTrue(body.contains("<details>"))
        XCTAssertTrue(body.contains("2 Error runtime/connect"))
        XCTAssertFalse(body.contains(NSHomeDirectory()))
    }

    func testLongSummaryIsTrimmedFromOldestProblemsToFitLinkLimit() {
        let problems = (1...200).map { "\($0) Warning runtime/reconnect result=" + String(repeating: "x", count: 80) }
        let summary = (["version=1.0.0", "records: runtime=200"] + problems).joined(separator: "\n")
        let url = SupportCenter.newIssueURL(environment: environment, error: String(repeating: "e", count: 5000),
                                            summary: summary, diagnostics: nil)
        XCTAssertLessThanOrEqual(url.absoluteString.count, SupportCenter.maximumIssueURLLength)
        let body = query(url)["body"] ?? ""
        XCTAssertTrue(body.contains("version=1.0.0"))
        XCTAssertTrue(body.contains("200 Warning runtime/reconnect"))
        XCTAssertFalse(body.contains("\n1 Warning runtime/reconnect"))
    }

    func testUnknownEnglishErrorPointsToFeedbackEntry() {
        XCTAssertTrue(friendlyError("unexpected internal state").contains("帮助 → 反馈问题"))
        XCTAssertEqual(friendlyError("请先连接已配对设备"), "请先连接已配对设备")
    }
}
