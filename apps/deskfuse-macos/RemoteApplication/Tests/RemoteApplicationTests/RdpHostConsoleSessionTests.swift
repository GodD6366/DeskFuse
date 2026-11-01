import Foundation
import Testing
@testable import DeskFuseRemoteApplication

@Suite("RDP host console session")
struct RdpHostConsoleSessionTests {
    @Test func oldServiceWithoutFieldDecodesAsUnknown() throws {
        let json = #"{"port":3389,"certificate_sha256":null,"checks":[]}"#
        let host = try JSONDecoder().decode(RemoteRdpHost.self, from: Data(json.utf8))
        #expect(host.consoleSession == nil)
        #expect(host.available == false)
    }

    @Test func decodesConnectedAndDisconnectedSessions() throws {
        let json = #"{"port":3390,"certificate_sha256":null,"checks":[],"console_session":{"user":"godd","connected":false}}"#
        let host = try JSONDecoder().decode(RemoteRdpHost.self, from: Data(json.utf8))
        #expect(host.consoleSession?.user == "godd")
        #expect(host.consoleSession?.connected == false)
    }

    @Test func encodesRoundTrip() throws {
        let host = RemoteRdpHost(
            port: 3389,
            certificateSha256: nil,
            checks: [],
            consoleSession: RemoteRdpConsoleSession(user: "中文用户", connected: true))
        let data = try JSONEncoder().encode(host)
        let decoded = try JSONDecoder().decode(RemoteRdpHost.self, from: data)
        #expect(decoded == host)
    }
}
