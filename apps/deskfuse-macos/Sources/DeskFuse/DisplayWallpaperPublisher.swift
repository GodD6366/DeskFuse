import AppKit
import CoreGraphics
import CoreFoundation
@preconcurrency import ScreenCaptureKit
import DeskFuseEngine

/// Capture only the visible wallpaper window for each display, never app
/// windows or desktop icons. A configured wallpaper image is the fallback when
/// the visible wallpaper surface is unavailable or ScreenCaptureKit returns black.
@MainActor final class DisplayWallpaperPublisher {
    struct Preview {
        let image: NSImage
        let jpeg: Data
    }

    struct RefreshResult {
        let previews: [String: Preview]
        let activeDisplayIDs: Set<String>
    }

    func refresh() async -> RefreshResult? {
        // Never trigger the system screen-recording prompt just to draw a preview.
        let content: SCShareableContent?
        if CGPreflightScreenCaptureAccess() {
            content = try? await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        } else {
            content = nil
        }
        guard !Task.isCancelled else { return nil }
        var changed: [String: Preview] = [:]
        var activeDisplayIDs = Set<String>()
        let wallpaperWindows = (content?.windows ?? []).filter { window in
            guard window.isOnScreen else { return false }
            let owner = window.owningApplication?.bundleIdentifier
            // WindowManager's title varies across macOS releases; its bundle
            // identity is the stable wallpaper surface.
            return owner == "com.apple.WindowManager"
                || (owner == "com.apple.dock" && window.title == "Desktop Picture")
        }
        for screen in NSScreen.screens {
            guard !Task.isCancelled,
                  let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else { continue }
            let displayID = CGDirectDisplayID(number.uint32Value)
            let bounds = CGDisplayBounds(displayID)
            guard bounds.width > 0, bounds.height > 0 else { continue }
            let uuid = CGDisplayCreateUUIDFromDisplayID(displayID).takeRetainedValue()
            let id = CFUUIDCreateString(nil, uuid) as String
            activeDisplayIDs.insert(id)
            // WindowManager can expose multiple full-screen surfaces. Do not
            // publish a black backing window simply because it was first.
            let candidates = wallpaperWindows.filter { window in
                  window.frame.intersection(bounds).area >= bounds.width * bounds.height * 0.9 &&
                  window.frame.width <= bounds.width * 1.1 &&
                  window.frame.height <= bounds.height * 1.1
            }.sorted {
                $0.frame.intersection(bounds).area > $1.frame.intersection(bounds).area
            }
            let scale = min(1, 960 / max(bounds.width, bounds.height))
            let configuration = SCStreamConfiguration()
            configuration.width = max(1, Int(bounds.width * scale))
            configuration.height = max(1, Int(bounds.height * scale))
            configuration.showsCursor = false
            var capturedImage: NSImage?
            for window in candidates {
                guard !Task.isCancelled else { return nil }
                guard let captured = try? await SCScreenshotManager.captureImage(
                    contentFilter: SCContentFilter(desktopIndependentWindow: window),
                    configuration: configuration
                ), !Self.isBlankCapture(captured) else { continue }
                capturedImage = NSImage(cgImage: captured, size: NSSize(width: captured.width, height: captured.height))
                break
            }
            if capturedImage == nil,
               let url = NSWorkspace.shared.desktopImageURL(for: screen) {
                capturedImage = NSImage(contentsOf: url)
            }
            guard !Task.isCancelled, let image = capturedImage else { continue }
            // ScreenCaptureKit and AppKit bridge through autoreleased objects.
            // Keep each display's temporary bitmap outside the loop lifetime;
            // only a changed preview escapes this scope.
            guard let preview: Preview = autoreleasepool(invoking: {
                guard let jpeg = Self.previewJPEG(image) else { return nil }
                return Preview(image: image, jpeg: jpeg)
            }) else { continue }
            changed[id] = preview
        }
        return RefreshResult(previews: changed, activeDisplayIDs: activeDisplayIDs)
    }

    /// Reject the empty/black surfaces returned before a wallpaper window is
    /// ready. A deliberately black configured image remains a valid fallback.
    static func isBlankCapture(_ image: CGImage) -> Bool {
        var pixels = [UInt8](repeating: 0, count: 16 * 16 * 4)
        return pixels.withUnsafeMutableBytes { bytes in
            guard let context = CGContext(data: bytes.baseAddress, width: 16, height: 16,
                bitsPerComponent: 8, bytesPerRow: 16 * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return true }
            context.draw(image, in: CGRect(x: 0, y: 0, width: 16, height: 16))
            let samples = bytes.bindMemory(to: UInt8.self)
            return stride(from: 0, to: samples.count, by: 4).allSatisfy { index in
                samples[index] <= 3 && samples[index + 1] <= 3 && samples[index + 2] <= 3
            }
        }
    }

    private static func previewJPEG(_ image: NSImage) -> Data? {
        // Prefer enough pixels for a Retina layout card, but keep each preview
        // within the input-service snapshot budget (16 screens at 64 KiB each).
        for maxWidth in [960.0, 800.0, 720.0, 640.0, 560.0, 480.0, 400.0, 320.0] {
            let jpeg: Data? = autoreleasepool(invoking: {
                let ratio = min(1, maxWidth / max(image.size.width, image.size.height))
                let width = max(1, Int(image.size.width * ratio))
                let height = max(1, Int(image.size.height * ratio))
                guard let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
                        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
                      let context = NSGraphicsContext(bitmapImageRep: bitmap) else { return nil }
                NSGraphicsContext.saveGraphicsState()
                NSGraphicsContext.current = context
                image.draw(in: NSRect(x: 0, y: 0, width: width, height: height))
                context.flushGraphics()
                NSGraphicsContext.restoreGraphicsState()
                for quality in [0.8, 0.7, 0.6] {
                    if let jpeg = bitmap.representation(using: .jpeg, properties: [.compressionFactor: quality]),
                       jpeg.count <= 65_536 { return jpeg }
                }
                return nil
            })
            if let jpeg { return jpeg }
        }
        return nil
    }
}

private extension CGRect {
    var area: CGFloat { isNull ? 0 : width * height }
}
