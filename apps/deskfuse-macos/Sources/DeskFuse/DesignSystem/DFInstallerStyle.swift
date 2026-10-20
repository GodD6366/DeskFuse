import SwiftUI

/// A spacious, single-action installation surface, separate from the dense workspace.
enum DFInstallerStyle {
    static let scale: CGFloat = 2.0 / 3.0
    static let layoutSize = CGSize(width: 600, height: 585)
    static let size = CGSize(width: 400, height: 390)
    static let inset: CGFloat = 52
    static let title = Font.system(size: 30, weight: .semibold)
    static let body = Font.system(size: 19)
    static let button = Font.system(size: 18, weight: .semibold)
    static let footer = Font.system(size: 14)
    static let radius: CGFloat = 9
    static let buttonHeight: CGFloat = 44
}
