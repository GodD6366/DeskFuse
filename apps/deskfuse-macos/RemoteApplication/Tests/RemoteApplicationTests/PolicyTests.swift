import Foundation
import Testing
@testable import DeskFuseRemoteApplication

@Test func nativeSixKIsNotCappedAndPortraitBudgetKeepsAspect() throws {
    let sixK = PixelSize(width: 6016, height: 3384)
    let profile = try VideoProfile.requested(native: sixK, sourceHz: 120, preferences: VideoPreferences())
    #expect(profile.pixels == sixK)
    #expect(profile.framesPerSecond == 120)
    #expect(profile.reason == nil)
    let portrait = ResolutionPreference.hd.dimensions(for: PixelSize(width: 2160, height: 3840))
    #expect(portrait == PixelSize(width: 1080, height: 1920))
    #expect(ResolutionPreference.uhd.dimensions(for: PixelSize(width: 800, height: 600)) == PixelSize(width: 800, height: 600))
}

@Test func sourceRefreshLimitIsExplained() throws {
    var preference = VideoPreferences(); preference.refresh = .hz120
    let profile = try VideoProfile.requested(native: PixelSize(width: 1920, height: 1080), sourceHz: 60, preferences: preference)
    #expect(profile.framesPerSecond == 60)
    #expect(profile.reason != nil)
    let unknown = try VideoProfile.requested(native: profile.pixels, sourceHz: nil, preferences: preference)
    #expect(unknown.framesPerSecond == 60)
    #expect(unknown.reason != nil)
}

@Test func codecPreferenceCarriesThroughRequestedProfile() throws {
    var preference = VideoPreferences()
    #expect(preference.codec == .automatic)
    #expect(try VideoProfile.requested(native: PixelSize(width: 1920, height: 1080), sourceHz: 60,
                                       preferences: preference, codec: .hevc).codec == .hevc)
    preference.codec = .h264
    #expect(try VideoProfile.requested(native: PixelSize(width: 1920, height: 1080), sourceHz: 60,
                                       preferences: preference).codec == .h264)
}

@Test func hevcCapabilitySelectionIsExplicit() {
    let hevc = MediaCapabilities(h264HardwareDecoder: true, hevcHardwareDecoder: true)
    let h264 = MediaCapabilities.h264Only
    #expect(hevc.supportsDecoder(.hevc))
    #expect(!h264.supportsDecoder(.hevc))
    #expect(h264.supportsDecoder(.automatic))
}

@Test func adaptationRequiresSustainedPressureAndRecoversOriginal() {
    let original = VideoProfile(pixels: PixelSize(width: 6016, height: 3384), framesPerSecond: 120)
    var adaptation = VideoAdaptation(profile: original, preference: .clarity)
    #expect({ adaptation.observe(congested: true) == nil }())
    #expect({ adaptation.observe(congested: true) == nil }())
    #expect({ adaptation.observe(congested: true) != nil }())
    #expect(adaptation.current.pixels == original.pixels)
    for _ in 0..<14 { #expect({ adaptation.observe(congested: false) == nil }()) }
    #expect({ adaptation.observe(congested: false) == original }())
    var fixed = VideoAdaptation(profile: original, preference: .fixed)
    for _ in 0..<100 { #expect({ fixed.observe(congested: true) == nil }()) }
    #expect(fixed.current == original)
}

@Test func clarityLowersRefreshBeforePixelsAndFluidityDoesTheReverse() {
    let profile = VideoProfile(pixels: PixelSize(width: 3840, height: 2160), framesPerSecond: 120, bitrate: 2_000_000)
    var clarity = VideoAdaptation(profile: profile, preference: .clarity)
    var fluidity = VideoAdaptation(profile: profile, preference: .fluidity)
    for _ in 0..<12 { _ = clarity.observe(congested: true); _ = fluidity.observe(congested: true) }
    #expect(clarity.current.pixels == profile.pixels)
    #expect(clarity.current.framesPerSecond < profile.framesPerSecond)
    #expect(fluidity.current.pixels.area < profile.pixels.area)
    #expect(fluidity.current.framesPerSecond == profile.framesPerSecond)
}

private func identity(_ generation: UInt64 = 1) -> SurfaceIdentity {
    SurfaceIdentity(session: UUID(uuidString: "00000000-0000-0000-0000-000000000001")!, epoch: 1, generation: generation, configuration: generation)
}

@Test func negativeOriginRetinaAndLetterboxCoordinates() throws {
    let surface = try CaptureSurface(identity: identity(), displayID: 7,
        displayFrame: CGRect(x: -1600, y: -200, width: 1600, height: 1000),
        contentRect: CGRect(x: -1500, y: -100, width: 1000, height: 700), pointPixelScale: 2, sourceHz: 60)
    #expect(surface.nativePixels == PixelSize(width: 2000, height: 1400))
    #expect(surface.displayLocalRect.origin == CGPoint(x: 100, y: 100))
    #expect(try surface.remotePoint(normalized: CGPoint(x: 0.5, y: 0.5)) == CGPoint(x: -1000, y: 250))
    #expect(CaptureSurface.normalized(viewPoint: CGPoint(x: 50, y: 5), videoRect: CGRect(x: 10, y: 20, width: 100, height: 100)) == nil)
    #expect(CaptureSurface.normalized(viewPoint: CGPoint(x: 60, y: 45), videoRect: CGRect(x: 10, y: 20, width: 100, height: 100)) == CGPoint(x: 0.5, y: 0.75))
}

@Test func cropFreezesDuringDragAndIncludesNewDialogAfterRelease() {
    var tracker = CropTracker()
    let display = CGRect(x: 0, y: 0, width: 1920, height: 1080)
    let primary = CGRect(x: 100, y: 100, width: 800, height: 600)
    let dialog = CGRect(x: 850, y: 50, width: 300, height: 300)
    #expect({ tracker.update(windows: [primary], display: display, gestureActive: false) == primary }())
    #expect({ tracker.update(windows: [primary, dialog], display: display, gestureActive: true) == primary }())
    #expect({ tracker.update(windows: [primary, dialog], display: display, gestureActive: false) == primary.union(dialog) }())
}

@Test func stalePresentationAndInputsCannotArmNewSurface() {
    var gate = SurfaceGate()
    gate.begin(identity())
    #expect({ !gate.accept(identity(), sequence: 1) }())
    #expect({ gate.acknowledge(identity()) }())
    #expect({ gate.accept(identity(), sequence: 1) }())
    #expect({ !gate.accept(identity(), sequence: 1) }())
    gate.begin(identity(2))
    #expect({ !gate.acknowledge(identity()) }())
    #expect({ !gate.accept(identity(2), sequence: 2) }())
    #expect({ gate.acknowledge(identity(2)) }())
    #expect({ gate.accept(identity(2), sequence: 1) }())
    gate.revoke()
    #expect({ !gate.accept(identity(2), sequence: 2) }())
}

private func frame(_ sequence: UInt64, key: Bool = false, generation: UInt64 = 1) -> EncodedVideoFrame {
    EncodedVideoFrame(surface: identity(generation), sequence: sequence, presentationNanoseconds: 0,
        keyFrame: key, sps: key ? Data([1]) : Data(), pps: key ? Data([2]) : Data(), bytes: Data([0, 0, 0, 1, 0x65]))
}

@Test func hevcKeyFrameRequiresVPSAndRejectsH264ParameterShape() {
    let surface = identity()
    let valid = EncodedVideoFrame(surface: surface, codec: .hevc, sequence: 1, presentationNanoseconds: 0,
        keyFrame: true, vps: Data([1]), sps: Data([2]), pps: Data([3]), bytes: Data([0, 0, 0, 1, 0x26]))
    #expect(throws: Never.self) { try valid.validate() }
    let missingVPS = EncodedVideoFrame(surface: surface, codec: .hevc, sequence: 1, presentationNanoseconds: 0,
        keyFrame: true, sps: Data([2]), pps: Data([3]), bytes: Data([0, 0, 0, 1, 0x26]))
    #expect(throws: RemoteApplicationError.self) { try missingVPS.validate() }
    let mixed = EncodedVideoFrame(surface: surface, codec: .h264, sequence: 1, presentationNanoseconds: 0,
        keyFrame: true, vps: Data([1]), sps: Data([2]), pps: Data([3]), bytes: Data([0, 0, 0, 1, 0x65]))
    #expect(throws: RemoteApplicationError.self) { try mixed.validate() }
}

@Test func missingReferenceRequiresKeyFrameInsteadOfDisplayingCorruption() {
    var queue = EncodedFrameQueue(capacity: 2); queue.reset(to: identity())
    #expect({ queue.offer(frame(1, key: true)) == .accepted }())
    #expect({ queue.offer(frame(3)) == .needsKeyFrame }())
    #expect(queue.count == 0)
    #expect({ queue.offer(frame(4)) == .needsKeyFrame }())
    #expect({ queue.offer(frame(5, key: true)) == .accepted }())
    #expect({ queue.offer(frame(6)) == .accepted }())
    #expect({ queue.offer(frame(7)) == .needsKeyFrame }())
    #expect(queue.count == 0)
    #expect({ queue.offer(frame(8, key: true)) == .accepted }())
    #expect({ queue.offer(frame(9, generation: 2)) == .stale }())
    #expect({ queue.take()?.sequence == 8 }())
}

@Test func malformedNALIsRejectedBeforeNativeDecoder() {
    let broken = EncodedVideoFrame(surface: identity(), sequence: 1, presentationNanoseconds: 0,
        keyFrame: false, bytes: Data([0, 0, 1, 0, 0x65]))
    #expect(throws: RemoteApplicationError.self) { try broken.validate() }
    #expect(throws: Never.self) { try frame(1, key: true).validate() }
}
