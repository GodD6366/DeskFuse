import AppKit
import Foundation
import Testing
@testable import DeskFuseRemoteApplication

@MainActor
@Suite("RDP clipboard DIB")
struct ClipboardImageTests {
    /// Builds a top-down RGBA image without touching AppKit color management.
    private func makeImage(_ pixels: [(r: UInt8, g: UInt8, b: UInt8)],
                           width: Int, height: Int) -> NSImage {
        var bytes: [UInt8] = []
        bytes.reserveCapacity(width * height * 4)
        for pixel in pixels {
            bytes.append(contentsOf: [pixel.r, pixel.g, pixel.b, 255])
        }
        let provider = CGDataProvider(data: Data(bytes) as CFData)!
        let cgImage = CGImage(
            width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32,
            bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
        return NSImage(cgImage: cgImage, size: NSSize(width: width, height: height))
    }

    /// Renders an NSImage back into top-down RGBA bytes for comparison.
    private func pixels(_ image: NSImage, width: Int, height: Int) -> [UInt8] {
        var buffer = [UInt8](repeating: 0, count: width * height * 4)
        guard let context = CGContext(
            data: &buffer, width: width, height: height, bitsPerComponent: 8,
            bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return [] }
        if let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) {
            context.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))
        }
        return buffer
    }

    private func u32(_ data: Data, _ offset: Int) -> UInt32 {
        data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: offset, as: UInt32.self) }.littleEndian
    }

    private func u16(_ data: Data, _ offset: Int) -> UInt16 {
        data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: offset, as: UInt16.self) }.littleEndian
    }

    private let red: (r: UInt8, g: UInt8, b: UInt8) = (255, 0, 0)
    private let green: (r: UInt8, g: UInt8, b: UInt8) = (0, 255, 0)
    private let blue: (r: UInt8, g: UInt8, b: UInt8) = (0, 0, 255)
    private let white: (r: UInt8, g: UInt8, b: UInt8) = (255, 255, 255)

    @Test func writesA32BitBottomUpHeaderAndBottomRowFirst() throws {
        // Top row red/green, bottom row blue/white.
        let image = makeImage([red, green, blue, white], width: 2, height: 2)
        let dib = try #require(RemoteRdpClipboard.encodeDIB(image))
        #expect(dib.count == 40 + 2 * 2 * 4)
        #expect(u32(dib, 0) == 40)
        #expect(u32(dib, 4) == 2)
        #expect(u32(dib, 8) == 2)          // positive height = bottom-up
        #expect(u16(dib, 12) == 1)
        #expect(u16(dib, 14) == 32)
        #expect(u32(dib, 16) == 0)         // BI_RGB
        #expect(u32(dib, 20) == 16)

        // First stored pixel row must be the image's bottom row, BGR order.
        #expect([UInt8](dib[40..<44]) == [255, 0, 0, 255])     // blue
        #expect([UInt8](dib[44..<48]) == [255, 255, 255, 255]) // white
    }

    @Test func roundTripsPixelsAndOrientation() throws {
        let image = makeImage([red, green, blue, white], width: 2, height: 2)
        let dib = try #require(RemoteRdpClipboard.encodeDIB(image))
        let decoded = try #require(RemoteRdpClipboard.decodeDIB(dib))
        #expect(decoded.size == NSSize(width: 2, height: 2))
        #expect(pixels(decoded, width: 2, height: 2) == pixels(image, width: 2, height: 2))
    }

    @Test func decodesTopDown24BitRows() throws {
        // 2×2 top-down 24bpp: red/green on top, blue/white below.
        let pixelBytes: [UInt8] = [
            0, 0, 255, 0, 255, 0, 0, 0,
            255, 0, 0, 255, 255, 255, 0, 0,
        ]
        var dib = Data()
        dib.append(contentsOf: [40, 0, 0, 0])
        dib.append(contentsOf: [2, 0, 0, 0])
        dib.append(contentsOf: [0xFE, 0xFF, 0xFF, 0xFF]) // -2: top-down
        dib.append(contentsOf: [1, 0, 24, 0])
        dib.append(contentsOf: [0, 0, 0, 0])
        dib.append(contentsOf: [16, 0, 0, 0])
        dib.append(contentsOf: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        dib.append(contentsOf: pixelBytes)
        let decoded = try #require(RemoteRdpClipboard.decodeDIB(dib))
        #expect(decoded.size == NSSize(width: 2, height: 2))
        #expect(pixels(decoded, width: 2, height: 2) == [
            255, 0, 0, 255, 0, 255, 0, 255,
            0, 0, 255, 255, 255, 255, 255, 255,
        ])
    }

    @Test func rejectsTruncatedCompressedAndPalettedDIBs() throws {
        let image = makeImage([red], width: 1, height: 1)
        let valid = try #require(RemoteRdpClipboard.encodeDIB(image))
        #expect(RemoteRdpClipboard.decodeDIB(valid.prefix(40)) == nil)

        var compressed = valid
        compressed.replaceSubrange(16..<20, with: [1, 0, 0, 0]) // BI_RLE8
        #expect(RemoteRdpClipboard.decodeDIB(compressed) == nil)

        var paletted = valid
        paletted.replaceSubrange(14..<16, with: [8, 0])
        #expect(RemoteRdpClipboard.decodeDIB(paletted) == nil)
    }
}
