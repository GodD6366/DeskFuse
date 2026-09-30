import CoreVideo
import Foundation
import Testing
@testable import DeskFuseRemoteApplication

@Suite struct RemoteTitlebarColorTests {
    @Test func darkBackgroundIgnoresTitleTextAndControls() throws {
        let buffer = try image(width: 320, height: 180) { x, y in
            if x < 60 || x > 260 || (140..<150).contains(x) { return (240, 240, 240, 255) }
            return y < 24 ? (26, 27, 38, 255) : (255, 255, 255, 255)
        }
        let color = try #require(RemoteTitlebarColor.sample(buffer: buffer, layout: nil))
        #expect(abs(color.red - 26.0 / 255) < 0.001)
        #expect(abs(color.green - 27.0 / 255) < 0.001)
        #expect(abs(color.blue - 38.0 / 255) < 0.001)
        #expect(color.isDark)
    }

    @Test func applicationCropUsesTopLeftCoordinatesAndEncodedScale() throws {
        let surface = try CaptureSurface(
            identity: SurfaceIdentity(session: UUID(), epoch: 1, generation: 1, configuration: 1),
            displayID: 1, displayFrame: CGRect(x: -1000, y: -200, width: 1000, height: 800),
            contentRect: CGRect(x: -900, y: -100, width: 400, height: 300),
            pointPixelScale: 2, sourceHz: 60,
            applicationRect: CGRect(x: -850, y: -50, width: 300, height: 200))
        let buffer = try image(width: 800, height: 600) { x, y in
            (100..<700).contains(x) && (100..<148).contains(y) ? (230, 232, 234, 255) : (10, 20, 30, 255)
        }
        let color = try #require(RemoteTitlebarColor.sample(buffer: buffer, layout: surface))
        #expect(abs(color.red - 230.0 / 255) < 0.001)
        #expect(abs(color.green - 232.0 / 255) < 0.001)
        #expect(abs(color.blue - 234.0 / 255) < 0.001)
        #expect(!color.isDark)
    }

    @Test func transparentFrameHasNoTitlebarColor() throws {
        let buffer = try image(width: 32, height: 32) { _, _ in (0, 0, 0, 0) }
        #expect(RemoteTitlebarColor.sample(buffer: buffer, layout: nil) == nil)
    }

    private func image(width: Int, height: Int,
                       pixel: (Int, Int) -> (UInt8, UInt8, UInt8, UInt8)) throws -> CVPixelBuffer {
        var buffer: CVPixelBuffer?
        let result = CVPixelBufferCreate(kCFAllocatorDefault, width, height, kCVPixelFormatType_32BGRA, nil, &buffer)
        #expect(result == kCVReturnSuccess)
        let image = try #require(buffer)
        #expect(CVPixelBufferLockBaseAddress(image, []) == kCVReturnSuccess)
        defer { CVPixelBufferUnlockBaseAddress(image, []) }
        let base = try #require(CVPixelBufferGetBaseAddress(image)).assumingMemoryBound(to: UInt8.self)
        let stride = CVPixelBufferGetBytesPerRow(image)
        for y in 0..<height {
            for x in 0..<width {
                let (r, g, b, a) = pixel(x, y)
                let offset = y * stride + x * 4
                base[offset] = b; base[offset + 1] = g; base[offset + 2] = r; base[offset + 3] = a
            }
        }
        return image
    }
}
