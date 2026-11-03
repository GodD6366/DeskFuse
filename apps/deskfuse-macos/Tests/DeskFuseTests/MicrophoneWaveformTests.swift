import Foundation
import XCTest
@testable import DeskFuse

/// Contract for the 2 s / 512-point rolling waveform window the microphone page
/// renders. The bridge projects exactly these fields to the WebView.
final class MicrophoneWaveformTests: XCTestCase {
    private let rate = 48_000.0

    private func sine(seconds: Double, frequency: Double = 440, amplitude: Float = 0.5) -> [Float] {
        let count = Int(seconds * rate)
        return (0..<count).map { index in
            Float(sin(2 * Double.pi * frequency * Double(index) / rate)) * amplitude
        }
    }

    private func uptimeMs() -> Double {
        Double(DispatchTime.now().uptimeNanoseconds) / 1_000_000
    }

    /// Feed 2 s of a 440 Hz sine: the window must hold exactly 512 points with
    /// sign-preserving peaks, span 2 s, a fresh timestamp and active == true.
    func testAppendTwoSecondsOfSineProducesRollingWindow() {
        let waveform = MicrophoneWaveform()
        let started = uptimeMs()
        waveform.append(sine(seconds: 2))

        let reading = waveform.current()
        XCTAssertEqual(reading.values.count, MicrophoneWaveform.pointCount)
        XCTAssertEqual(reading.values.count, 512)
        XCTAssertEqual(reading.span, 2.0, accuracy: 1e-9)
        XCTAssertEqual(reading.rate, 48_000)
        XCTAssertTrue(reading.active)

        let peak = reading.values.lazy.map { abs($0) }.max() ?? 0
        XCTAssertEqual(peak, 0.5, accuracy: 0.002, "每个 187.5 样本桶都包含接近正弦峰值的样本")
        XCTAssertTrue(reading.values.contains { $0 < -0.49 }, "峰值抽样必须保留符号")

        let ended = uptimeMs()
        XCTAssertGreaterThan(reading.endMs, 0)
        XCTAssertGreaterThanOrEqual(reading.endMs, started)
        XCTAssertLessThanOrEqual(reading.endMs, ended)
        XCTAssertLessThan(ended - reading.endMs, 1_000, "endMs 应是最近一次 append 的时刻")
    }

    /// The window is a rolling one: appending more audio drops the oldest points.
    func testAppendKeepsOnlyNewestPoints() {
        let waveform = MicrophoneWaveform()
        waveform.append(sine(seconds: 2, frequency: 110))
        XCTAssertTrue(waveform.current().active)
        waveform.clear()

        var values = [Float](repeating: 0, count: 960)
        values[800] = 0.75
        waveform.append(values)
        let reading = waveform.current()
        XCTAssertEqual(reading.values.count, 512)
        // 960 samples produce 5 points; the impulse at 800 lands in one of them.
        XCTAssertEqual(reading.values.lazy.map { abs($0) }.max() ?? 0, 0.75, accuracy: 1e-6)
    }

    /// A receiver stores the sender's window verbatim, with the local receive
    /// time as the end timestamp.
    func testAcceptStoresRemoteWindowWithReceiveTimestamp() {
        let waveform = MicrophoneWaveform()
        let window = (0..<512).map { index in
            Float(sin(Double(index) * 2 * Double.pi * 440 / 48_000)) * 0.5
        }
        let receivedAfter = uptimeMs()
        waveform.accept(window, span: 2.0)

        let reading = waveform.current()
        XCTAssertEqual(reading.values, window)
        XCTAssertEqual(reading.span, 2.0, accuracy: 1e-9)
        XCTAssertEqual(reading.rate, 48_000)
        XCTAssertTrue(reading.active)
        XCTAssertGreaterThanOrEqual(reading.endMs, receivedAfter)
        XCTAssertLessThanOrEqual(reading.endMs, uptimeMs())
    }

    /// Shorter remote windows are left-padded with zeros (oldest history missing),
    /// longer ones keep their newest 512 points.
    func testAcceptPadsAndTruncatesKeepingNewestSamples() {
        let waveform = MicrophoneWaveform()
        let short = (0..<100).map { Float($0) / 100 }
        waveform.accept(short, span: 1.0)
        let padded = waveform.current()
        XCTAssertEqual(padded.values.count, 512)
        XCTAssertEqual(Array(padded.values.prefix(412)), [Float](repeating: 0, count: 412))
        XCTAssertEqual(Array(padded.values.suffix(100)), short)
        XCTAssertEqual(padded.span, 1.0, accuracy: 1e-9)

        let long = (0..<600).map { Float($0) }
        waveform.accept(long, span: 2.0)
        let truncated = waveform.current()
        XCTAssertEqual(truncated.values.count, 512)
        XCTAssertEqual(truncated.values, Array(long.suffix(512)))
    }

    /// Clearing the session must stop the animation and reset the window.
    func testClearResetsWindowAndActivity() {
        let waveform = MicrophoneWaveform()
        waveform.accept(sine(seconds: 2).prefix(512).map { $0 }, span: 2.0)
        XCTAssertTrue(waveform.current().active)

        waveform.clear()
        let reading = waveform.current()
        XCTAssertFalse(reading.active)
        XCTAssertEqual(reading.values, [Float](repeating: 0, count: 512))
        XCTAssertEqual(reading.span, 2.0, accuracy: 1e-9)
        XCTAssertEqual(reading.endMs, 0)
    }
}

/// The WebView contract for the microphone page projection. The frontend renderer
/// scrolls by real time, so these names and units must not drift.
@MainActor
final class MicrophoneBridgeProjectionTests: XCTestCase {
    func testProjectionExposesContinuousWindowAndTimestamp() throws {
        defer {
            MicrophoneSharing.shared.waveform.clear()
            MicrophoneSharing.shared.snapshot = nil
        }
        let state = """
        {"settings":{"enabled":true,"source":"","allowed":[],"input_uid":""},
         "phase":"streaming","detail":"","session_id":7,"role":"sender","peer":"","source_name":"",
         "native":{"driver":true,"readers":false,"streaming":true,"locked":false,
                   "permission":"authorized","level":0.25,"error":""}}
        """
        MicrophoneSharing.shared.snapshot = try JSONDecoder().decode(
            MicrophoneSharingSnapshot.self, from: Data(state.utf8))
        let waveform = MicrophoneSharing.shared.waveform
        waveform.append((0..<96_000).map { Float(sin(Double($0) * 0.05)) * 0.5 })

        let projection = MyGoSnapshot.microphoneProjection(MicrophoneSharing.shared, live: false)
        let native = try XCTUnwrap(projection["native"] as? [String: Any])
        let values = try XCTUnwrap(native["waveform"] as? [Float])
        XCTAssertEqual(values.count, 512, "投影必须恰好给出 512 个点")
        XCTAssertEqual(native["waveform_span_ms"] as? Int, 2000)
        XCTAssertEqual(native["waveform_rate"] as? Double, 48_000)
        XCTAssertEqual(native["waveform_active"] as? Bool, true)
        let age = try XCTUnwrap(native["waveform_age_ms"] as? Int)
        XCTAssertGreaterThanOrEqual(age, 0)
        XCTAssertLessThan(age, 500, "刚写入的样本不应报成旧数据")
    }
}
