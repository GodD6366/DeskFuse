import Foundation
import CoreVideo
import CoreGraphics
import CoreMedia
import DeskFuseRemoteApplication

private final class Errors: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String] = []
    func append(_ error: RemoteApplicationError) { lock.withLock { if values.count < 20 { values.append(error.localizedDescription) } } }
    func snapshot() -> [String] { lock.withLock { values } }
}

@main struct Probe {
    static func main() async {
        do {
            let arguments = Array(CommandLine.arguments.dropFirst())
            if arguments.contains("--help") {
                print("deskfuse-media-probe [--codec h264|hevc] [--width 1920] [--height 1080] [--fps 60] [--seconds 3]\nSynthetic local hardware codec probe. No screen capture, input, QUIC, or input-to-photon claim.")
                return
            }
            func option(_ name: String, _ fallback: Int) throws -> Int {
                guard let index = arguments.firstIndex(of: name) else { return fallback }
                guard index + 1 < arguments.count, let value = Int(arguments[index + 1]), value > 0 else {
                    throw RemoteApplicationError.unsupported("Invalid \(name)")
                }
                return value
            }
            let size = PixelSize(width: try option("--width", 1920), height: try option("--height", 1080))
            let fps = try option("--fps", 60), seconds = try option("--seconds", 3)
            let codec: VideoCodec
            if let index = arguments.firstIndex(of: "--codec"), index + 1 < arguments.count {
                switch arguments[index + 1].lowercased() {
                case "h264": codec = .h264
                case "hevc", "h265": codec = .hevc
                default: throw RemoteApplicationError.unsupported("Invalid --codec; use h264 or hevc")
                }
            } else { codec = .h264 }
            guard size.isValid, size.width % 2 == 0, size.height % 2 == 0, fps <= 120, seconds <= 600 else {
                throw RemoteApplicationError.unsupported("Invalid dimensions, fps, or duration")
            }
            let surface = SurfaceIdentity(session: UUID(), epoch: 1, generation: 1, configuration: 1)
            let profile = VideoProfile(codec: codec, pixels: size, framesPerSecond: Double(fps))
            let errors = Errors()
            let pipeline = try LocalMediaPipeline(surface: surface, profile: profile) { errors.append($0) }
            defer { pipeline.stop() }
            var pool: CVPixelBufferPool?
            let attributes: [CFString: Any] = [kCVPixelBufferWidthKey: size.width, kCVPixelBufferHeightKey: size.height,
                kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
                kCVPixelBufferIOSurfacePropertiesKey: [:] as [String: Any], kCVPixelBufferCGImageCompatibilityKey: true]
            let allocated = CVPixelBufferPoolCreate(kCFAllocatorDefault, nil, attributes as CFDictionary, &pool)
            guard allocated == kCVReturnSuccess, let pool else { throw RemoteApplicationError.codec("Create pixel pool", allocated) }
            var start = ProcessInfo.processInfo.systemUptime
            // Codec initialization is measured separately from steady state.
            // Drain the one-second warmup before resetting measurement counters.
            for index in 0..<(fps * (seconds + 1)) {
                if index == fps {
                    try await Task.sleep(for: .milliseconds(250))
                    pipeline.metrics.resetMeasurement()
                    start = ProcessInfo.processInfo.systemUptime - 1
                }
                var image: CVPixelBuffer?
                let result = CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(kCFAllocatorDefault, pool,
                    [kCVPixelBufferPoolAllocationThresholdKey: 8] as CFDictionary, &image)
                guard result == kCVReturnSuccess, let image else { throw RemoteApplicationError.codec("Allocate frame", result) }
                CVPixelBufferLockBaseAddress(image, [])
                if let context = CGContext(data: CVPixelBufferGetBaseAddress(image), width: size.width, height: size.height,
                    bitsPerComponent: 8, bytesPerRow: CVPixelBufferGetBytesPerRow(image), space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.noneSkipFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue) {
                    context.setFillColor(CGColor(gray: 0.08, alpha: 1)); context.fill(CGRect(x: 0, y: 0, width: size.width, height: size.height))
                    context.setFillColor(CGColor(red: Double(index % 60) / 60, green: 0.8, blue: 0.3, alpha: 1))
                    context.fill(CGRect(x: (index * 17) % max(1, size.width - 80), y: 0, width: 80, height: size.height))
                }
                CVPixelBufferUnlockBaseAddress(image, [])
                pipeline.metrics.update { $0.captured += 1 }
                pipeline.submit(CapturedVideoFrame(surface: surface, buffer: image, timestamp: CMTime(value: Int64(index), timescale: Int32(fps))))
                let delay = start + Double(index + 1) / Double(fps) - ProcessInfo.processInfo.systemUptime
                if delay > 0 { try await Task.sleep(for: .seconds(delay)) }
            }
            // Bounded drain, not a throughput extension.
            for _ in 0..<100 {
                if pipeline.metrics.snapshot().decoded >= UInt64(fps * seconds) { break }
                try await Task.sleep(for: .milliseconds(10))
            }
            let stats = pipeline.metrics.snapshot()
            struct Report: Encodable {
                let scope: String; let profile: VideoProfile; let durationSeconds: Int
                let warmupSeconds: Int; let decodedFramesPerSecond: Double
                let statistics: MediaStatistics; let errors: [String]; let hardwareSmokePassed: Bool
                let meetsRequestedCodecRate: Bool
            }
            let passed = stats.hardwareEncoder && stats.hardwareDecoder && errors.snapshot().isEmpty && stats.decoded > 0
            let decodedFPS = Double(stats.decoded) / Double(seconds)
            let report = Report(scope: "local synthetic hardware encode/decode; no network, capture, display, or latency acceptance",
                profile: profile, durationSeconds: seconds, warmupSeconds: 1, decodedFramesPerSecond: decodedFPS,
                statistics: stats, errors: errors.snapshot(), hardwareSmokePassed: passed, meetsRequestedCodecRate: decodedFPS >= Double(fps) - 1)
            let encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            print(String(decoding: try encoder.encode(report), as: UTF8.self))
            if !passed { exit(1) }
        } catch {
            FileHandle.standardError.write(Data((error.localizedDescription + "\n").utf8)); exit(1)
        }
    }
}
