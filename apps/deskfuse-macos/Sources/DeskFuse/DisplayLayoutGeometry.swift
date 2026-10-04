import Foundation
import DeskFuseEngine

/// Freeze this transform for the entire drag/commit, even if the view resizes
/// or engine snapshots arrive. Gesture translations are in canvas coordinates.
struct DisplayLayoutViewport: Equatable {
    let scale: CGFloat
    let origin: CGPoint
    let layoutOrigin: CGPoint

    init(displays: [Display], size: CGSize) {
        let minX = displays.map(\.x).min() ?? 0
        let minY = displays.map(\.y).min() ?? 0
        let maxX = displays.map { $0.x + $0.width }.max() ?? 1
        let maxY = displays.map { $0.y + $0.height }.max() ?? 1
        scale = max(0.001, min((size.width - 64) / max(1, maxX - minX),
                               (size.height - 64) / max(1, maxY - minY)))
        origin = CGPoint(x: (size.width - (maxX - minX) * scale) / 2,
                         y: (size.height - (maxY - minY) * scale) / 2)
        layoutOrigin = CGPoint(x: minX, y: minY)
    }

    func position(of display: Display, translation: CGSize = .zero) -> CGPoint {
        CGPoint(x: origin.x + (display.x - layoutOrigin.x) * scale + translation.width,
                y: origin.y + (display.y - layoutOrigin.y) * scale + translation.height)
    }

    func logicalTranslation(_ translation: CGSize) -> CGSize {
        CGSize(width: translation.width / scale, height: translation.height / scale)
    }
}

/// A submission receipt or a readiness-only snapshot is not a layout commit.
/// Compare the whole arrangement, allowing the engine to rebase its origin.
enum DisplayLayoutConfirmation {
    static func matches(before: [Display], after: [Display], deviceID: String, translation: CGSize) -> Bool {
        let before = before.sorted { $0.layoutID < $1.layoutID }
        let after = after.sorted { $0.layoutID < $1.layoutID }
        guard !before.isEmpty, before.count == after.count,
              before.contains(where: { $0.deviceId == deviceID }),
              zip(before, after).allSatisfy({ $0.layoutID == $1.layoutID
                  && $0.width == $1.width && $0.height == $1.height }) else { return false }
        func expected(_ display: Display) -> CGPoint {
            CGPoint(x: display.x + (display.deviceId == deviceID ? translation.width : 0),
                    y: display.y + (display.deviceId == deviceID ? translation.height : 0))
        }
        let base = expected(before[0]), actualBase = CGPoint(x: after[0].x, y: after[0].y)
        return zip(before, after).allSatisfy { original, actual in
            let desired = expected(original)
            return abs((desired.x - base.x) - (actual.x - actualBase.x)) < 0.5
                && abs((desired.y - base.y) - (actual.y - actualBase.y)) < 0.5
        }
    }
}
