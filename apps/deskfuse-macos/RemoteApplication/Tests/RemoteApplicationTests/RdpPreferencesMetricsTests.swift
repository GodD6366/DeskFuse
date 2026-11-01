import Foundation
import Testing
@testable import DeskFuseRemoteApplication

@Suite("RDP preferences")
struct RdpPreferencesTests {
    private func defaults() -> UserDefaults {
        let suite = "RemoteRdpPreferencesTests." + UUID().uuidString
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        return defaults
    }

    @Test func missingKeysDefaultToEnabled() {
        let preferences = RemoteRdpPreferences.load(from: defaults())
        #expect(preferences.audioEnabled)
        #expect(preferences.clipboardEnabled)
    }

    @Test func roundTripsExplicitValues() {
        let defaults = defaults()
        RemoteRdpPreferences(audioEnabled: false, clipboardEnabled: true).save(to: defaults)
        #expect(RemoteRdpPreferences.load(from: defaults) == RemoteRdpPreferences(audioEnabled: false, clipboardEnabled: true))
    }

    @Test func disabledAudioStaysDisabled() {
        let defaults = defaults()
        RemoteRdpPreferences(audioEnabled: false, clipboardEnabled: false).save(to: defaults)
        let loaded = RemoteRdpPreferences.load(from: defaults)
        #expect(!loaded.audioEnabled)
        #expect(!loaded.clipboardEnabled)
    }
}

@Suite("RDP session metrics")
struct RdpSessionMetricsTests {
    private let start = Date(timeIntervalSince1970: 1_000)

    @Test func tracksConnectAndFirstFrameOnce() {
        var metrics = RemoteRdpSessionMetrics()
        metrics.begin(at: start)
        #expect(metrics.isActive)
        #expect(metrics.connectMilliseconds() == nil)

        metrics.markConnected(at: start.addingTimeInterval(0.25))
        metrics.markConnected(at: start.addingTimeInterval(0.5)) // ignored
        #expect(metrics.connectMilliseconds() == 250)

        let firstFrame = metrics.markFrame(at: start.addingTimeInterval(0.5))
        #expect(firstFrame)
        let secondFrame = metrics.markFrame(at: start.addingTimeInterval(0.6))
        #expect(!secondFrame)
        #expect(metrics.firstFrameMilliseconds() == 500)
        #expect(metrics.frames == 2)
    }

    @Test func computesUptimeAndFrameRate() {
        var metrics = RemoteRdpSessionMetrics()
        metrics.begin(at: start)
        for index in 1...10 {
            _ = metrics.markFrame(at: start.addingTimeInterval(Double(index) * 0.1))
        }
        #expect(metrics.uptimeMilliseconds(now: start.addingTimeInterval(2)) == 2_000)
        #expect(abs(metrics.frameRate(now: start.addingTimeInterval(2)) - 5) < 0.0001)
    }

    @Test func inactiveSessionHasNoTimings() {
        let metrics = RemoteRdpSessionMetrics()
        #expect(!metrics.isActive)
        #expect(metrics.uptimeMilliseconds(now: start) == nil)
        #expect(metrics.frameRate(now: start) == 0)
    }

    @Test func beginResetsPreviousSession() {
        var metrics = RemoteRdpSessionMetrics()
        metrics.begin(at: start)
        _ = metrics.markFrame(at: start.addingTimeInterval(0.1))
        metrics.begin(at: start.addingTimeInterval(5))
        #expect(metrics.frames == 0)
        #expect(metrics.firstFrameMilliseconds() == nil)
    }
}
