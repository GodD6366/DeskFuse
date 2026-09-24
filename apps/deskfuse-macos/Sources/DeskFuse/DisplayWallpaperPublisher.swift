import AppKit
import CoreGraphics
import CoreFoundation
@preconcurrency import ScreenCaptureKit
import DeskFuseEngine

/// Capture only the visible wallpaper window for each display, never app
/// windows or desktop icons. NSWorkspace.desktopImageURL can describe a
/// different Space's configured image instead of the wallpaper on screen.
@MainActor final class DisplayWallpaperPublisher {
    struct Preview {
        let image: NSImage
        let jpeg: Data
    }

    private var publishedJPEG: [String: Data] = [:]

    func refresh() async -> [String: Preview] {
        // Never trigger the system screen-recording prompt just to draw a preview.
        guard CGPreflightScreenCaptureAccess(),
              let content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true) else {
            return [:]
        }
        var changed: [String: Preview] = [:]
        let wallpaperWindows = content.windows.filter { window in
            guard window.isOnScreen else { return false }
            let owner = window.owningApplication?.bundleIdentifier
            return (owner == "com.apple.WindowManager" && window.title == "Wallpaper")
                || (owner == "com.apple.dock" && window.title == "Desktop Picture")
        }
        for display in content.displays {
            let bounds = display.frame
            guard bounds.width > 0, bounds.height > 0,
                  let window = wallpaperWindows.max(by: {
                      $0.frame.intersection(bounds).area < $1.frame.intersection(bounds).area
                  }),
                  window.frame.intersection(bounds).area >= bounds.width * bounds.height * 0.9,
                  window.frame.width <= bounds.width * 1.1,
                  window.frame.height <= bounds.height * 1.1 else { continue }
            let uuid = CGDisplayCreateUUIDFromDisplayID(display.displayID).takeRetainedValue()
            let id = CFUUIDCreateString(nil, uuid) as String
            let scale = min(1, 960 / max(bounds.width, bounds.height))
            let configuration = SCStreamConfiguration()
            configuration.width = max(1, Int(bounds.width * scale))
            configuration.height = max(1, Int(bounds.height * scale))
            configuration.showsCursor = false
            guard let captured = try? await SCScreenshotManager.captureImage(
                contentFilter: SCContentFilter(desktopIndependentWindow: window),
                configuration: configuration
            ) else { continue }
            let image = NSImage(cgImage: captured, size: NSSize(width: captured.width, height: captured.height))
            guard let jpeg = Self.previewJPEG(image) else { continue }
            if publishedJPEG[id] != jpeg {
                publishedJPEG[id] = jpeg
                changed[id] = Preview(image: image, jpeg: jpeg)
            }
        }
        return changed
    }

    private static func previewJPEG(_ image: NSImage) -> Data? {
        // Prefer enough pixels for a Retina layout card, but keep each preview
        // within the input-service snapshot budget (16 screens at 64 KiB each).
        for maxWidth in [960.0, 800.0, 720.0, 640.0, 560.0, 480.0, 400.0, 320.0] {
            let ratio = min(1, maxWidth / max(image.size.width, image.size.height))
            let width = max(1, Int(image.size.width * ratio))
            let height = max(1, Int(image.size.height * ratio))
            guard let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
                    bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                    colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
                  let context = NSGraphicsContext(bitmapImageRep: bitmap) else { continue }
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = context
            image.draw(in: NSRect(x: 0, y: 0, width: width, height: height))
            context.flushGraphics()
            NSGraphicsContext.restoreGraphicsState()
            for quality in [0.8, 0.7, 0.6] {
                if let jpeg = bitmap.representation(using: .jpeg, properties: [.compressionFactor: quality]),
                   jpeg.count <= 65_536 { return jpeg }
            }
        }
        return nil
    }
}

private extension CGRect {
    var area: CGFloat { isNull ? 0 : width * height }
}
