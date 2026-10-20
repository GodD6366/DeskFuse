import Foundation

/// Wire constants, not Quartz button numbers. Kept stable in synced settings.
enum LogitechHIDProtocol {
    static let gesture = 0xc3
    static let modeShift = 0xc4
    static let mxButtons: Set<Int> = [gesture, modeShift]
    // Separate namespace for physical Button Spy indices, not CIDs or Quartz buttons.
    static let g502Buttons = Set(208..<224)
    static let buttons = mxButtons.union(g502Buttons)

    enum Model: String, CaseIterable, Hashable, Identifiable {
        case g502Lightspeed
        case mxMaster3

        var id: String { rawValue }
        var displayName: String {
            switch self {
            case .g502Lightspeed: "G502 LIGHTSPEED"
            case .mxMaster3: "MX Master 3 / 3S"
            }
        }
        var supportSummary: String {
            switch self {
            case .g502Lightspeed: "11 个实体键 · 保留原生功能"
            case .mxMaster3: "手势键、模式切换键与标准按键"
            }
        }
    }
    static func model(_ name: String) -> Model? {
        let name = name.lowercased()
        if name.contains("mx master 3") { return .mxMaster3 }
        if name.contains("g502 lightspeed") { return .g502Lightspeed }
        return nil
    }

    static func spyButtons(count: Int) -> Set<Int>? {
        guard (1...16).contains(count) else { return nil }
        return Set(208..<(208 + count))
    }

    static func spyPressed(_ payload: [UInt8], count: Int) -> Set<Int>? {
        guard payload.count >= 2, let available = spyButtons(count: count) else { return nil }
        let bitmap = UInt16(payload[0]) << 8 | UInt16(payload[1])
        return Set(available.filter { bitmap & (UInt16(1) << ($0 - 208)) != 0 })
    }

    /// Ignore unrelated vendor collections (e.g. the receiver's ffbc mouse HID).
    /// A HID++ long-report endpoint must expose report 0x11 in both directions.
    static func hasLongReports(_ descriptor: [UInt8]) -> Bool {
        var index = 0, reportID = 0, size = 0, count = 0
        var stack: [(Int, Int, Int)] = []
        var input = 0, output = 0
        while index < descriptor.count {
            let prefix = descriptor[index]; index += 1
            if prefix == 0xfe {
                guard index + 2 <= descriptor.count else { return false }
                let length = Int(descriptor[index]); index += 2
                guard index + length <= descriptor.count else { return false }
                index += length; continue
            }
            let length = [0, 1, 2, 4][Int(prefix & 3)]
            guard index + length <= descriptor.count else { return false }
            var value = 0
            for offset in 0..<length { value |= Int(descriptor[index + offset]) << (8 * offset) }
            index += length
            switch prefix & 0xfc {
            case 0x74: size = value
            case 0x84: reportID = value
            case 0x94: count = value
            case 0xa4: stack.append((reportID, size, count))
            case 0xb4:
                guard let saved = stack.popLast() else { return false }
                (reportID, size, count) = saved
            case 0x80 where reportID == 0x11:
                input += min(size, 4096) * min(count, 4096)
            case 0x90 where reportID == 0x11:
                output += min(size, 4096) * min(count, 4096)
            default: break
            }
        }
        // The report ID itself is not included in ReportCount.
        return input >= 19 * 8 && output >= 19 * 8
    }
    static func error(_ bytes: [UInt8], matching expected: [UInt8]) -> UInt8? {
        guard expected.count == 3, bytes.count >= 5, bytes[0] == expected[0],
              bytes[1] == 0xff || bytes[1] == 0x8f,
              bytes[2] == expected[1], bytes[3] == expected[2] else { return nil }
        return bytes[4]
    }
    static func supportedModel(_ name: String) -> Bool {
        model(name) != nil
    }
    static func control(_ payload: [UInt8]) -> Int? {
        // getCidInfo: CID(2), task(2), flags, position, group, mask, flags2.
        guard payload.count >= 8, payload[4] & 0x20 != 0,
              payload[4] & 0x80 == 0 else { return nil }
        let cid = Int(payload[0]) << 8 | Int(payload[1])
        return mxButtons.contains(cid) ? cid : nil
    }
    static func reporting(_ cid: Int, divert: Bool) -> [UInt8] {
        // Change temporary diversion only. The valid bit is required even to clear it.
        [UInt8(cid >> 8), UInt8(cid & 255), divert ? 3 : 2, 0, 0]
    }
    static func pressed(_ payload: [UInt8]) -> Set<Int>? {
        guard payload.count >= 8 else { return nil }
        return Set(stride(from: 0, to: 8, by: 2).map {
            Int(payload[$0]) << 8 | Int(payload[$0 + 1])
        }.filter { $0 != 0 })
    }
    static func report(_ bytes: [UInt8], reportID: UInt32) -> [UInt8]? {
        guard reportID == 0x10 || reportID == 0x11 else { return nil }
        let size = reportID == 0x10 ? 7 : 20
        // IOHID normally includes report ID; accept the explicitly ID-stripped form too.
        if bytes.count == size, bytes.first == UInt8(reportID) { return Array(bytes.dropFirst()) }
        if bytes.count == size - 1 { return bytes }
        return nil
    }
}
