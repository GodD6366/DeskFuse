import Foundation
import SwiftUI

struct ClipboardCodeDocument {
    let language: String
    let text: String

    static func detect(_ source: String) -> Self? {
        // Keep recognition bounded for very large clipboard records.
        let sample = String(source.prefix(50_000)).trimmingCharacters(in: .whitespacesAndNewlines)
        if sample.hasPrefix("```"), let newline = source.firstIndex(of: "\n"),
           let closing = source.range(of: "```", options: .backwards), closing.lowerBound > newline {
            let label = source[..<newline].trimmingCharacters(in: .whitespacesAndNewlines)
                .replacingOccurrences(of: "```", with: "")
            return Self(language: label.isEmpty ? "代码" : label.uppercased(),
                        text: String(source[source.index(after: newline)..<closing.lowerBound]))
        }
        if sample.hasPrefix("{") || sample.hasPrefix("[") {
            if source.utf8.count <= 200_000,
               let value = try? JSONSerialization.jsonObject(with: Data(source.utf8)),
               let data = try? JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .withoutEscapingSlashes]),
               let formatted = String(data: data, encoding: .utf8) {
                return Self(language: "JSON", text: formatted)
            }
            if sample.range(of: #"[\{,]\s*"[^"\n]+"\s*:"#, options: .regularExpression) != nil {
                return Self(language: "JSON", text: source)
            }
        }
        let signatures: [(String, String)] = [
            ("Python", #"(?m)^\s*(?:def\s+\w+\([^\n]*\)\s*(?:->[^\n]+)?:|from\s+[\w.]+\s+import\s+\w+|class\s+\w+(?:\([^\n]*\))?:)"#),
            ("Swift", #"(?m)^\s*(?:func\s+\w+\(|(?:struct|enum|class)\s+\w+[^\n]*\{|import\s+(?:SwiftUI|AppKit|Foundation)\b)"#),
            ("JavaScript / TypeScript", #"(?m)^\s*(?:(?:export\s+)?(?:async\s+)?function\s+\w+\(|(?:const|let|var)\s+\w+\s*=|import\s+.+\s+from\s+['"])"#),
            ("Rust", #"(?m)^\s*(?:(?:pub\s+)?(?:async\s+)?fn\s+\w+\(|use\s+\w+::|impl\s+\w+[^\n]*\{)"#),
            ("SQL", #"(?is)^\s*(?:SELECT\s+.+\s+FROM\s+|INSERT\s+INTO\s+|CREATE\s+TABLE\s+|UPDATE\s+\w+\s+SET\s+)"#),
            ("HTML / XML", #"(?is)^\s*(?:<!DOCTYPE\s+html|<\?xml|<(?:html|div|span|body|svg)\b[^>]*>)"#),
            ("代码", #"(?m)^\s*(?:#include\s*[<"]|(?:public|private|protected)\s+(?:static\s+)?(?:class|void|int|string)\s+\w+|#!/(?:usr/)?bin/)"#)
        ]
        for (language, pattern) in signatures where sample.range(of: pattern, options: .regularExpression) != nil {
            return Self(language: language, text: source)
        }
        return nil
    }

    var highlighted: AttributedString {
        var result = AttributedString(text)
        let sample = String(text.prefix(50_000))
        let pattern = #"("(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*')|(//[^\n]*|/\*[\s\S]*?\*/|(?m:^\s*#[^\n]*))|\b(true|false|null|nil|None|True|False|const|let|var|func|fn|def|class|struct|enum|import|from|return|if|else|for|while|async|await|try|throw|pub|use|SELECT|FROM|WHERE|INSERT|INTO)\b|\b(\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\b"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return result }
        for match in regex.matches(in: sample, range: NSRange(sample.startIndex..., in: sample)) {
            guard let range = Range(match.range, in: text),
                  let lower = AttributedString.Index(range.lowerBound, within: result),
                  let upper = AttributedString.Index(range.upperBound, within: result) else { continue }
            let color: Color = match.range(at: 1).location != NSNotFound ? DFPalette.success
                : match.range(at: 2).location != NSNotFound ? Color.secondary
                : match.range(at: 3).location != NSNotFound ? DFPalette.accent : DFPalette.warning
            result[lower..<upper].foregroundColor = color
        }
        return result
    }
}

struct ClipboardCodePreview: View {
    let document: ClipboardCodeDocument

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Label(document.language, systemImage: "chevron.left.forwardslash.chevron.right")
                .font(DFType.detail.weight(.medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 16).padding(.vertical, 10)
            Divider()
            ScrollView([.horizontal, .vertical]) {
                Text(document.highlighted)
                    .font(DFType.mono)
                    .lineSpacing(4)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: true, vertical: true)
                    .padding(18)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .background(Color(nsColor: .textBackgroundColor))
    }
}
