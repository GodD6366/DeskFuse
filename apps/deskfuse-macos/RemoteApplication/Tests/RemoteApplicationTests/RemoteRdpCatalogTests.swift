import Foundation
import Testing
@testable import DeskFuseRemoteApplication

private let fingerprint = (0..<32).map { String(format: "%02X", $0) }.joined(separator: ":")

private func hostJSON(port: UInt16 = 3389, certificate: String? = fingerprint,
                      checks: String = #"[{"name":"edition","passed":true,"message":"ok"}]"#) -> String {
    #"{"port":\#(port),"certificate_sha256":\#(certificate.map { "\"\($0)\"" } ?? "null"),"checks":\#(checks)}"#
}

@Test func catalogV2CarriesHostPreflightAndApps() throws {
    let json = #"{"type":"catalog","version":2,"available":true,"host":\#(hostJSON()),"apps":[{"name":"notepad","display_name":"Notepad","path":"C:\\Windows\\notepad.exe","command_line":"","icon_path":"","icon_index":0}]}"#
    let message = try JSONDecoder().decode(RemoteRdpMessage.self, from: Data(json.utf8))
    guard case .catalog(let version, let available, _, let host, let apps) = message else {
        Issue.record("expected catalog"); return
    }
    #expect(version == 2)
    #expect(available)
    #expect(host.port == 3389)
    #expect(host.available)
    #expect(host.certificateSha256 == fingerprint)
    #expect(apps.map(\.name) == ["notepad"])
}

@Test func oldCatalogVersionIsRejectedInsteadOfSilentlyDowngraded() {
    let json = #"{"type":"catalog","version":1,"available":true,"apps":[]}"#
    #expect(throws: RemoteRdpProtocolError.unsupportedVersion(1)) {
        try JSONDecoder().decode(RemoteRdpMessage.self, from: Data(json.utf8))
    }
}

@Test func newerCatalogVersionReportsPeerVersion() {
    let json = #"{"type":"catalog","version":3,"available":true,"host":\#(hostJSON()),"apps":[]}"#
    #expect(throws: RemoteRdpProtocolError.unsupportedVersion(3)) {
        try JSONDecoder().decode(RemoteRdpMessage.self, from: Data(json.utf8))
    }
}

@Test func catalogWithoutHostIsAFormatError() {
    let json = #"{"type":"catalog","version":2,"available":true,"apps":[]}"#
    #expect(throws: DecodingError.self) {
        try JSONDecoder().decode(RemoteRdpMessage.self, from: Data(json.utf8))
    }
}

@Test func readyCarriesSingleUseTicketAndPinnedHost() throws {
    let ticket = (0..<32).map(String.init).joined(separator: ",")
    let json = #"{"type":"ready","version":2,"name":"notepad","ticket":[\#(ticket)],"rdp":"full address:s:deskfuse-rdp.invalid","host":\#(hostJSON())}"#
    let message = try JSONDecoder().decode(RemoteRdpMessage.self, from: Data(json.utf8))
    guard case .ready(let version, let name, let data, let rdp, let host) = message else {
        Issue.record("expected ready"); return
    }
    #expect(version == 2)
    #expect(name == "notepad")
    #expect(data.count == 32)
    #expect(rdp.contains("deskfuse-rdp.invalid"))
    #expect(host.certificateSha256 == fingerprint)
}

@Test func readyRejectsWrongTicketLength() {
    let json = #"{"type":"ready","version":2,"name":"notepad","ticket":[1,2,3],"rdp":"x","host":\#(hostJSON())}"#
    #expect(throws: DecodingError.self) {
        try JSONDecoder().decode(RemoteRdpMessage.self, from: Data(json.utf8))
    }
}

@Test func prepareRequestEncodesVersionTwo() throws {
    let data = try JSONEncoder().encode(RemoteRdpRequest.prepare(version: 2, name: "notepad"))
    let value = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    #expect(value["type"] as? String == "prepare")
    #expect(value["version"] as? Int == 2)
    #expect(value["name"] as? String == "notepad")
}

@Test func hostReadinessRequiresChecksAndAValidPin() {
    let passed = RemoteRdpHostCheck(name: "edition", passed: true, message: "ok")
    let failed = RemoteRdpHostCheck(name: "rdp_enabled", passed: false, message: "off")
    #expect(RemoteRdpHost(port: 3389, certificateSha256: fingerprint, checks: [passed]).available)
    #expect(!RemoteRdpHost(port: 0, certificateSha256: fingerprint, checks: [passed]).available)
    #expect(!RemoteRdpHost(port: 3389, certificateSha256: fingerprint, checks: [passed, failed]).available)
    #expect(!RemoteRdpHost(port: 3389, certificateSha256: nil, checks: [passed]).available)
    #expect(!RemoteRdpHost(port: 3389, certificateSha256: "00:11", checks: [passed]).available)
    #expect(!RemoteRdpHost.isValidFingerprint(fingerprint.lowercased().replacingOccurrences(of: ":", with: "")))
    #expect(RemoteRdpHost(port: 3389, certificateSha256: fingerprint, checks: [passed, failed]).failedChecks == [failed])
}
