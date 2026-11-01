import Foundation
import Testing
@testable import DeskFuseRemoteApplication

@Suite("RDP app catalog icons")
struct RdpAppIconTests {
    private func catalogJSON(icon: String?) -> Data {
        let iconField = icon.map { #","icon_png":"\#($0)""# } ?? ""
        let json = #"{"name":"notepad","display_name":"Notepad","path":"C:\\Windows\\notepad.exe","command_line":"","icon_path":"","icon_index":0\#(iconField)}"#
        return Data(json.utf8)
    }

    @Test func decodesBase64Icon() throws {
        let app = try JSONDecoder().decode(RemoteRdpApp.self, from: catalogJSON(icon: "aGVsbG8="))
        #expect(app.iconPng == "aGVsbG8=")
        #expect(app.iconPng.flatMap { Data(base64Encoded: $0) } == Data("hello".utf8))
    }

    @Test func missingIconStaysNil() throws {
        let app = try JSONDecoder().decode(RemoteRdpApp.self, from: catalogJSON(icon: nil))
        #expect(app.iconPng == nil)
    }

    @Test func invalidBase64DoesNotDecodeIntoImageData() throws {
        let app = try JSONDecoder().decode(RemoteRdpApp.self, from: catalogJSON(icon: "not base64!"))
        #expect(app.iconPng == "not base64!")
        #expect(app.iconPng.flatMap { Data(base64Encoded: $0) } == nil)
    }
}
