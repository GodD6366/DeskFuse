import AppKit
import Foundation

/// 富文本转换留在原生适配层；不导入可触发外部资源读取的网页 HTML。
@MainActor enum MyGoRichText {
    struct Run: Codable, Sendable {
        let text: String
        let bold: Bool?
        let italic: Bool?
        let underline: Bool?
        let strike: Bool?
    }

    static func html(from rtf: Data) -> String? {
        guard !rtf.isEmpty, let value = NSAttributedString(rtf: rtf, documentAttributes: nil),
              let data = try? value.data(from: NSRange(location: 0, length: value.length),
                  documentAttributes: [.documentType: NSAttributedString.DocumentType.html]) else { return nil }
        return String(data: data, encoding: .utf8)
    }

    static func runs(from rtf: Data) -> [Run]? {
        guard !rtf.isEmpty, let value = NSAttributedString(rtf: rtf, documentAttributes: nil) else { return nil }
        var runs: [Run] = []
        value.enumerateAttributes(in: NSRange(location: 0, length: value.length)) { attributes, range, _ in
            let traits = (attributes[.font] as? NSFont).map { NSFontManager.shared.traits(of: $0) } ?? []
            runs.append(Run(text: (value.string as NSString).substring(with: range),
                bold: traits.contains(.boldFontMask), italic: traits.contains(.italicFontMask),
                underline: ((attributes[.underlineStyle] as? NSNumber)?.intValue ?? 0) != 0,
                strike: ((attributes[.strikethroughStyle] as? NSNumber)?.intValue ?? 0) != 0))
        }
        return runs
    }

    static func mutation(id: UInt64, text: String, runs: [Run]?) throws -> String {
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              text.count <= 1_000_000 else { throw NSError(domain: "MyGoRichText", code: 1, userInfo: [NSLocalizedDescriptionKey: "文本为空或超过长度限制"]) }
        var request: [String: Any] = ["op": "set_item_text", "id": NSNumber(value: id), "text": text]
        if let runs {
            guard runs.count <= 10_000, runs.map(\.text).joined() == text else {
                throw NSError(domain: "MyGoRichText", code: 2, userInfo: [NSLocalizedDescriptionKey: "文本与样式内容不一致"])
            }
            let value = NSMutableAttributedString(string: "")
            var styled = false
            for run in runs {
                var traits: NSFontTraitMask = []
                if run.bold == true { traits.insert(.boldFontMask) }
                if run.italic == true { traits.insert(.italicFontMask) }
                let font = NSFontManager.shared.convert(NSFont.systemFont(ofSize: 15), toHaveTrait: traits)
                var attributes: [NSAttributedString.Key: Any] = [.font: font]
                if run.underline == true { attributes[.underlineStyle] = NSUnderlineStyle.single.rawValue }
                if run.strike == true { attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue }
                styled = styled || !traits.isEmpty || run.underline == true || run.strike == true
                value.append(NSAttributedString(string: run.text, attributes: attributes))
            }
            if styled, let rtf = value.rtf(from: NSRange(location: 0, length: value.length)) { request["rtf"] = Array(rtf) }
        }
        return String(decoding: try JSONSerialization.data(withJSONObject: request), as: UTF8.self)
    }
}
