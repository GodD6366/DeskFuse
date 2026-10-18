import SwiftUI

struct ClipboardSelectionFrames: PreferenceKey {
    static let defaultValue: [UInt64: CGRect] = [:]

    static func reduce(value: inout [UInt64: CGRect], nextValue: () -> [UInt64: CGRect]) {
        value.merge(nextValue(), uniquingKeysWith: { _, new in new })
    }
}

extension View {
    func clipboardSelectionFrame(_ id: UInt64) -> some View {
        background {
            GeometryReader { geometry in
                Color.clear.preference(key: ClipboardSelectionFrames.self,
                                       value: [id: geometry.frame(in: .named("clipboard-selection"))])
            }
        }
    }
}
