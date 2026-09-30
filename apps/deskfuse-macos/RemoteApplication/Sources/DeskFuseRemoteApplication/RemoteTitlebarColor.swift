import CoreVideo
import Foundation

/// A sparse sample of the captured titlebar, independent of the viewer's theme.
public struct RemoteTitlebarColor: Equatable, Sendable {
    public let red: Double
    public let green: Double
    public let blue: Double

    public var isDark: Bool {
        func linear(_ value: Double) -> Double {
            value <= 0.04045 ? value / 12.92 : pow((value + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue) < 0.179
    }

    public static func sample(buffer: CVPixelBuffer, layout: CaptureSurface?) -> Self? {
        guard CVPixelBufferGetPixelFormatType(buffer) == kCVPixelFormatType_32BGRA,
              CVPixelBufferLockBaseAddress(buffer, .readOnly) == kCVReturnSuccess else { return nil }
        defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(buffer) else { return nil }
        let width = CVPixelBufferGetWidth(buffer), height = CVPixelBufferGetHeight(buffer)
        guard width > 0, height > 0 else { return nil }
        let source = layout?.contentRect ?? CGRect(x: 0, y: 0, width: width, height: height)
        let application = layout?.applicationRect ?? source
        guard source.width > 0, source.height > 0, !application.isEmpty else { return nil }
        let stride = CVPixelBufferGetBytesPerRow(buffer)
        let bytes = base.assumingMemoryBound(to: UInt8.self)
        // Ignore corners, traffic lights and edge controls. Use the dominant
        // color bucket rather than averaging title text/icons into the background.
        var buckets: [Int: (count: Int, red: Int, green: Int, blue: Int)] = [:]
        var winner = 0, maximum = 0
        for row in 0..<3 {
            let logicalY = application.minY + min(application.height * 0.08, 24) * Double(row + 1) / 4
            let y = min(height - 1, max(0, Int((logicalY - source.minY) / source.height * Double(height))))
            for column in 0..<32 {
                let logicalX = application.minX + application.width * (0.2 + 0.6 * (Double(column) + 0.5) / 32)
                let x = min(width - 1, max(0, Int((logicalX - source.minX) / source.width * Double(width))))
                let offset = y * stride + x * 4
                guard bytes[offset + 3] >= 240 else { continue }
                let b = Int(bytes[offset]), g = Int(bytes[offset + 1]), r = Int(bytes[offset + 2])
                let key = (r >> 4) << 8 | (g >> 4) << 4 | (b >> 4)
                let old = buckets[key] ?? (0, 0, 0, 0)
                let next = (count: old.count + 1, red: old.red + r, green: old.green + g, blue: old.blue + b)
                buckets[key] = next
                if next.count > maximum { winner = key; maximum = next.count }
            }
        }
        guard let color = buckets[winner], color.count > 0 else { return nil }
        let divisor = Double(color.count) * 255
        return Self(red: Double(color.red) / divisor, green: Double(color.green) / divisor, blue: Double(color.blue) / divisor)
    }
}
