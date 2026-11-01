import AppKit
import Foundation

/// Bridges the macOS pasteboard to an RDP session's cliprdr channel.
///
/// Text travels as Windows `CF_UNICODETEXT` (UTF-16LE with a terminating NUL);
/// images travel as `CF_DIB` (BITMAPINFOHEADER, 24/32-bit BI_RGB, bottom-up rows).
/// File lists remain unsupported and are never announced, so Windows never waits
/// for an answer this class cannot produce.
@available(macOS 14.2, *)
@MainActor
public final class RemoteRdpClipboard {
    public static let unicodeTextFormat: UInt32 = 13
    public static let dibFormat: UInt32 = 8

    /// One pasteboard generation: the formats we advertise and their payloads.
    struct Snapshot {
        let formats: [UInt32]
        let data: [UInt32: Data]
    }

    private weak var bridge: RemoteRdpBridge?
    private var timer: Timer?
    private var lastChangeCount = 0
    private var pendingFormat: UInt32?

    public init() {}

    /// Starts watching the pasteboard and publishes its current contents so the
    /// Windows side can paste immediately after connecting.
    public func attach(to bridge: RemoteRdpBridge) {
        detach()
        self.bridge = bridge
        lastChangeCount = NSPasteboard.general.changeCount
        bridge.setClipboardHandler { [weak self] event in
            Task { @MainActor in self?.handle(event) }
        }
        let timer = Timer(timeInterval: 0.4, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.poll() }
        }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
        publishCurrentClipboard()
    }

    public func detach() {
        timer?.invalidate()
        timer = nil
        bridge?.setClipboardHandler(nil)
        bridge = nil
        pendingFormat = nil
    }

    private func poll() {
        let pasteboard = NSPasteboard.general
        guard pasteboard.changeCount != lastChangeCount else { return }
        lastChangeCount = pasteboard.changeCount
        publishCurrentClipboard()
    }

    private func publishCurrentClipboard() {
        guard let bridge else { return }
        let snapshot = Self.snapshot()
        bridge.clipboardSetFormats(snapshot.formats)
        for (format, data) in snapshot.data {
            bridge.clipboardSetData(format: format, data: data)
        }
    }

    private func handle(_ event: RemoteRdpBridge.ClipboardEvent) {
        guard let bridge else { return }
        switch event {
        case .ready:
            publishCurrentClipboard()
        case .formats(let ids):
            guard pendingFormat == nil else { return }
            guard let format = [Self.unicodeTextFormat, Self.dibFormat].first(where: ids.contains) else { return }
            pendingFormat = format
            bridge.clipboardRequest(format: format)
        case .data(let format, let payload):
            guard format == pendingFormat else { return }
            pendingFormat = nil
            guard let payload else { return }
            switch format {
            case Self.unicodeTextFormat:
                if let text = Self.decodeText(payload) { write(text) }
            case Self.dibFormat:
                if let image = Self.decodeDIB(payload) { write(image) }
            default:
                break
            }
        case .request(let format):
            // Answer with data only; re-announcing here can bounce a second
            // request back from Windows.
            if let data = Self.snapshot().data[format] {
                bridge.clipboardSetData(format: format, data: data)
            }
        }
    }

    private func write(_ text: String) {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
        lastChangeCount = pasteboard.changeCount
    }

    private func write(_ image: NSImage) {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.writeObjects([image])
        lastChangeCount = pasteboard.changeCount
    }

    /// Text wins when both are present (a browser selection can carry both);
    /// otherwise a raster image is advertised as DIB.
    static func snapshot(of pasteboard: NSPasteboard = .general) -> Snapshot {
        if let text = pasteboard.string(forType: .string), !text.isEmpty,
           let data = encodeText(text) {
            return Snapshot(formats: [unicodeTextFormat], data: [unicodeTextFormat: data])
        }
        if let image = NSImage(pasteboard: pasteboard), let data = encodeDIB(image) {
            return Snapshot(formats: [dibFormat], data: [dibFormat: data])
        }
        return Snapshot(formats: [], data: [:])
    }

    // MARK: - Text

    /// UTF-16LE without BOM, terminated by one NUL code unit.
    nonisolated static func encodeText(_ text: String) -> Data? {
        var data = Data()
        data.reserveCapacity(text.utf16.count * 2 + 2)
        for unit in text.utf16 {
            data.append(UInt8(unit & 0xff))
            data.append(UInt8(unit >> 8))
        }
        data.append(0)
        data.append(0)
        return data
    }

    /// Stops at the first NUL and ignores a trailing half code unit.
    nonisolated static func decodeText(_ data: Data) -> String? {
        guard data.count >= 2 else { return nil }
        let bytes = [UInt8](data)
        var units: [UInt16] = []
        units.reserveCapacity(bytes.count / 2)
        var index = 0
        while index + 1 < bytes.count {
            let unit = UInt16(bytes[index]) | (UInt16(bytes[index + 1]) << 8)
            if unit == 0 { break }
            units.append(unit)
            index += 2
        }
        guard !units.isEmpty else { return nil }
        return String(decoding: units, as: UTF16.self)
    }

    // MARK: - DIB images

    private nonisolated static let maxPixels = 8_388_608  // 32 MiB at 32 bpp

    /// Encodes an image as a 32-bit BI_RGB DIB with bottom-up rows.
    ///
    /// Rendering goes through a DeviceRGB `CGContext` with RGBA bytes so the
    /// channel order and row direction do not depend on `NSBitmapImageRep`'s
    /// internal layout.
    nonisolated static func encodeDIB(_ image: NSImage) -> Data? {
        let size = image.size
        guard size.width > 0, size.height > 0 else { return nil }
        let scale = min(1, 4096 / max(size.width, size.height))
        let width = max(1, Int((size.width * scale).rounded()))
        let height = max(1, Int((size.height * scale).rounded()))
        guard width <= 4096, height <= 4096, width * height <= maxPixels else { return nil }
        guard let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil) else { return nil }

        var buffer = [UInt8](repeating: 0, count: width * height * 4)
        guard let context = CGContext(
            data: &buffer, width: width, height: height, bitsPerComponent: 8,
            bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else { return nil }
        context.interpolationQuality = .high
        context.draw(cgImage, in: CGRect(x: 0, y: 0, width: width, height: height))

        var data = Data(capacity: 40 + width * height * 4)
        append(UInt32(40), to: &data)
        append(Int32(width), to: &data)
        append(Int32(height), to: &data)     // positive: bottom-up
        append(UInt16(1), to: &data)
        append(UInt16(32), to: &data)
        append(UInt32(0), to: &data)         // BI_RGB
        append(UInt32(width * height * 4), to: &data)
        append(Int32(2835), to: &data)
        append(Int32(2835), to: &data)
        append(UInt32(0), to: &data)
        append(UInt32(0), to: &data)

        // CGContext keeps row 0 at the top; DIB rows start at the bottom.
        for row in stride(from: height - 1, through: 0, by: -1) {
            let base = row * width * 4
            for column in 0..<width {
                let pixel = base + column * 4
                data.append(buffer[pixel + 2]) // B
                data.append(buffer[pixel + 1]) // G
                data.append(buffer[pixel])     // R
                data.append(255)
            }
        }
        return data
    }

    /// Accepts uncompressed 24/32-bit BI_RGB DIBs, the shape Windows uses for
    /// clipboard images. Compressed, paletted and bitfield variants are rejected
    /// rather than guessed at. Alpha is treated as opaque: Windows commonly
    /// stores 0 in the unused byte of a 32-bit clipboard DIB.
    nonisolated static func decodeDIB(_ data: Data) -> NSImage? {
        guard data.count >= 40,
              let headerSize = readUInt32(data, 0), headerSize >= 40,
              Int(headerSize) <= data.count,
              let widthValue = readUInt32(data, 4),
              let heightValue = readUInt32(data, 8),
              let planes = readUInt16(data, 12),
              let bitCount = readUInt16(data, 14),
              let compression = readUInt32(data, 16) else { return nil }

        let width = Int(Int32(bitPattern: widthValue))
        let rawHeight = Int(Int32(bitPattern: heightValue))
        let height = abs(rawHeight)
        guard planes == 1, compression == 0, (bitCount == 24 || bitCount == 32),
              width > 0, width <= 8192, height > 0, height <= 8192,
              width * height <= maxPixels else { return nil }

        let stride = ((width * Int(bitCount) + 31) / 32) * 4
        let start = Int(headerSize)
        guard stride > 0, data.count >= start + stride * height else { return nil }

        var rgba = Data(count: width * height * 4)
        let topDown = rawHeight < 0
        data.withUnsafeBytes { source in
            rgba.withUnsafeMutableBytes { destination in
                guard let src = source.baseAddress, let dst = destination.baseAddress else { return }
                for row in 0..<height {
                    let sourceRow = topDown ? row : height - 1 - row
                    let srcRow = src.advanced(by: start + sourceRow * stride)
                    let dstRow = dst.advanced(by: row * width * 4)
                    for column in 0..<width {
                        let offset = column * (Int(bitCount) / 8)
                        let b = srcRow.load(fromByteOffset: offset, as: UInt8.self)
                        let g = srcRow.load(fromByteOffset: offset + 1, as: UInt8.self)
                        let r = srcRow.load(fromByteOffset: offset + 2, as: UInt8.self)
                        let out = dstRow.advanced(by: column * 4)
                        out.storeBytes(of: r, toByteOffset: 0, as: UInt8.self)
                        out.storeBytes(of: g, toByteOffset: 1, as: UInt8.self)
                        out.storeBytes(of: b, toByteOffset: 2, as: UInt8.self)
                        out.storeBytes(of: 255, toByteOffset: 3, as: UInt8.self)
                    }
                }
            }
        }
        guard let provider = CGDataProvider(data: rgba as CFData),
              let cgImage = CGImage(
                width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32,
                bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
                provider: provider, decode: nil, shouldInterpolate: true,
                intent: .defaultIntent) else { return nil }
        return NSImage(cgImage: cgImage, size: NSSize(width: width, height: height))
    }

    private nonisolated static func append<T: FixedWidthInteger>(_ value: T, to data: inout Data) {
        var little = value.littleEndian
        withUnsafeBytes(of: &little) { data.append(contentsOf: $0) }
    }

    private nonisolated static func readUInt32(_ data: Data, _ offset: Int) -> UInt32? {
        guard offset >= 0, offset + 4 <= data.count else { return nil }
        return data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: offset, as: UInt32.self) }.littleEndian
    }

    private nonisolated static func readUInt16(_ data: Data, _ offset: Int) -> UInt16? {
        guard offset >= 0, offset + 2 <= data.count else { return nil }
        return data.withUnsafeBytes { $0.loadUnaligned(fromByteOffset: offset, as: UInt16.self) }.littleEndian
    }
}
