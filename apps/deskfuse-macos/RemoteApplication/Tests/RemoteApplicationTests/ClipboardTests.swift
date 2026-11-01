import Foundation
import Testing
@testable import DeskFuseRemoteApplication

@Suite("RDP clipboard text conversion")
struct ClipboardTests {
    @Test func encodesUTF16LittleEndianWithTerminatingNUL() {
        let data = RemoteRdpClipboard.encodeText("A中")
        #expect(data == Data([0x41, 0x00, 0x2D, 0x4E, 0x00, 0x00]))
    }

    @Test func roundTripsBMPAndSurrogatePairs() throws {
        for text in ["plain", "中文输入", "emoji 🙂 与拉丁", "a\tb\r\nc"] {
            let encoded = try #require(RemoteRdpClipboard.encodeText(text))
            #expect(RemoteRdpClipboard.decodeText(encoded) == text)
        }
    }

    @Test func stopsAtFirstNUL() throws {
        var data = try #require(RemoteRdpClipboard.encodeText("first"))
        data.append(contentsOf: [0x73, 0x00])
        #expect(RemoteRdpClipboard.decodeText(data) == "first")
    }

    @Test func ignoresTrailingHalfCodeUnit() throws {
        var data = try #require(RemoteRdpClipboard.encodeText("ok"))
        data.append(0x41)
        #expect(RemoteRdpClipboard.decodeText(data) == "ok")
    }

    @Test func rejectsEmptyAndNULOnlyPayloads() {
        #expect(RemoteRdpClipboard.decodeText(Data()) == nil)
        #expect(RemoteRdpClipboard.decodeText(Data([0x00, 0x00])) == nil)
        #expect(RemoteRdpClipboard.decodeText(Data([0x41])) == nil)
    }
}
