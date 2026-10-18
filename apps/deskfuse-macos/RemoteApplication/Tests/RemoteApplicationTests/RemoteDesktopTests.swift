import Foundation
import Testing
@testable import DeskFuseRemoteApplication

@Test func remoteDesktopReservationsRejectDuplicateProcessAndEnforceBudget() throws {
    var reservations = RemoteDesktopReservations()
    let owner = UUID()
    for pid: Int32 in 1...4 { try reservations.reserve(processID: pid, owner: owner, limit: 4) }
    #expect(throws: (any Error).self) { try reservations.reserve(processID: 1, owner: UUID(), limit: 4) }
    #expect(throws: (any Error).self) { try reservations.reserve(processID: 5, owner: owner, limit: 4) }
    reservations.release(processID: 1, owner: UUID())
    #expect(throws: (any Error).self) { try reservations.reserve(processID: 5, owner: owner, limit: 4) }
    reservations.release(processID: 1, owner: owner)
    try reservations.reserve(processID: 5, owner: owner, limit: 4)
}

@Test func remoteDesktopLateCleanupCannotReleaseReplacement() throws {
    var reservations = RemoteDesktopReservations()
    let old = UUID(), next = UUID()
    try reservations.reserve(processID: 1, owner: old, limit: 1)
    reservations.release(processID: 1, owner: old)
    try reservations.reserve(processID: 1, owner: next, limit: 1)
    reservations.release(processID: 1, owner: old)
    #expect(throws: (any Error).self) { try reservations.reserve(processID: 2, owner: old, limit: 1) }
    reservations.release(processID: 1, owner: next)
    try reservations.reserve(processID: 2, owner: old, limit: 1)
}

@Test func remoteDesktopQuartzConversionHandlesAboveAndNegativeOrigins() {
    let cocoa = CGRect(x: -1920, y: 1080, width: 1920, height: 1055)
    #expect(RemoteDesktopGeometry.quartzFrame(cocoa, primaryHeight: 1080)
        == CGRect(x: -1920, y: -1055, width: 1920, height: 1055))
}

@Test func remoteDesktopPinnedCaptureNeverFallsBackToPhysicalDisplay() {
    let physical = CaptureDisplayChoice(id: 1, frame: CGRect(x: 0, y: 0, width: 1920, height: 1080))
    let virtual = CaptureDisplayChoice(id: 2, frame: CGRect(x: 1920, y: 0, width: 1920, height: 1080))
    #expect(CaptureDisplayChoice.select([physical, virtual], pinned: 2, previous: 1, windows: [physical.frame]) == 2)
    #expect(CaptureDisplayChoice.select([physical], pinned: 2, previous: 1, windows: [physical.frame]) == nil)
    #expect(CaptureDisplayChoice.select([physical, virtual], pinned: nil, previous: nil, windows: [virtual.frame]) == 2)
    #expect(CaptureDisplayChoice.select([physical, virtual], pinned: nil, previous: 1, windows: [virtual.frame]) == 1)
}

@Test func remoteDesktopIgnoresFinderDesktopAndNonMovableAXElements() {
    #expect(RemoteDesktopWindowPolicy.shouldManage(role: "AXWindow", movable: true, desktopSurface: false))
    #expect(!RemoteDesktopWindowPolicy.shouldManage(role: "AXScrollArea", movable: true, desktopSurface: false))
    #expect(!RemoteDesktopWindowPolicy.shouldManage(role: "AXWindow", movable: false, desktopSurface: false))
    #expect(!RemoteDesktopWindowPolicy.shouldManage(role: "AXWindow", movable: true, desktopSurface: true))
}

@Test func remoteDesktopPlacementSurvivesDockAndMinimumSizeChanges() {
    let display = CGRect(x: 1920, y: 0, width: 1920, height: 1080)
    let previousWorkArea = CGRect(x: 1920, y: 25, width: 1920, height: 1055)
    let activeWorkArea = CGRect(x: 1920, y: 25, width: 1920, height: 985)
    #expect(!RemoteDesktopGeometry.contains(previousWorkArea, in: activeWorkArea))
    #expect(RemoteDesktopGeometry.isPlaced(previousWorkArea, on: display))
    #expect(RemoteDesktopGeometry.isPlaced(CGRect(x: 1920, y: 0, width: 1920, height: 1200), on: display))
    #expect(!RemoteDesktopGeometry.isPlaced(CGRect(x: 0, y: 25, width: 1200, height: 700), on: display))
}

@Test func remoteDesktopActivationOnlyChangesLayoutNotInputGeometry() throws {
    let identity = SurfaceIdentity(session: UUID(), epoch: 1, generation: 1, configuration: 1)
    let display = CGRect(x: 3840, y: 0, width: 1920, height: 1080)
    let before = CGRect(x: 3840, y: 0, width: 1920, height: 1080)
    let after = CGRect(x: 3840, y: 25, width: 1920, height: 985)
    let first = try CaptureSurface(identity: identity, displayID: 3, displayFrame: display,
        contentRect: CaptureDisplayChoice.captureRect(display: display, application: before, pinned: true),
        pointPixelScale: 1, sourceHz: 60, applicationRect: before)
    let second = try CaptureSurface(identity: identity, displayID: 3, displayFrame: display,
        contentRect: CaptureDisplayChoice.captureRect(display: display, application: after, pinned: true),
        pointPixelScale: 1, sourceHz: 60, applicationRect: after)
    #expect(first.hasSameCaptureGeometry(as: second))
    #expect(first.applicationRect != second.applicationRect)
    #expect(CaptureDisplayChoice.captureRect(display: display, application: after, pinned: false) == after)
}

@Test func remoteDesktopHandoffDrainsCaptureAndBlocksReconfigurationUntilArmed() {
    var gate = CaptureControlInterlock()
    #expect({ gate.beginCapture() }())
    let first = UUID(), second = UUID()
    gate.beginControl(first)
    #expect(!gate.canControl(first))
    #expect({ !gate.beginCapture() }())
    gate.endCapture()
    #expect(gate.canControl(first))
    #expect({ !gate.beginCapture() }())
    gate.beginControl(second)
    gate.endControl(first)
    #expect(!gate.canControl(first))
    #expect(gate.canControl(second))
    #expect({ !gate.beginCapture() }())
    gate.endControl(second)
    #expect({ gate.beginCapture() }())
    gate.endCapture()
    gate.beginControl(first)
    gate.cancelControl()
    #expect(!gate.canControl(first))
    #expect({ gate.beginCapture() }())
}

@Test func remoteDesktopFitKeepsDialogsInsideDedicatedScreen() {
    let screen = CGRect(x: 2560, y: 25, width: 1920, height: 1055)
    let dialog = CGRect(x: 20, y: 30, width: 480, height: 200)
    let fitted = RemoteDesktopGeometry.fit(dialog, inside: screen)
    #expect(fitted == CGRect(x: 2560, y: 30, width: 480, height: 200))
    #expect(RemoteDesktopGeometry.contains(fitted, in: screen))
    #expect(!RemoteDesktopGeometry.contains(dialog, in: screen))
    let oversized = RemoteDesktopGeometry.fit(CGRect(x: -3000, y: -1000, width: 6000, height: 4000), inside: screen)
    #expect(oversized == CGRect(x: 2560, y: 25, width: 1582.5, height: 1055))
    #expect(RemoteDesktopGeometry.contains(oversized, in: screen))
    #expect(!RemoteDesktopGeometry.contains(.zero, in: screen))
}

@Test func remoteDesktopWindowEnumerationGraceIsBoundedAndResets() {
    var grace = CaptureWindowAvailabilityGrace()
    #expect(!grace.isRecovering)
    #expect({ grace.allowsRetry(at: 10) }())
    #expect(grace.isRecovering)
    #expect({ grace.allowsRetry(at: 12.99) }())
    #expect({ !grace.allowsRetry(at: 13) }())
    #expect({ !grace.allowsRetry(at: 20) }())
    grace.reset()
    #expect(!grace.isRecovering)
    #expect({ grace.allowsRetry(at: 20) }())
    #expect({ !grace.allowsRetry(at: 23) }())
}

@Test func remoteDesktopPinnedEmptyWindowEnumerationKeepsDedicatedBoundary() {
    let physical = CaptureDisplayChoice(id: 1, frame: CGRect(x: 0, y: 0, width: 1920, height: 1080))
    let virtual = CaptureDisplayChoice(id: 2, frame: CGRect(x: 1920, y: 0, width: 1920, height: 1080))
    #expect(CaptureDisplayChoice.select([physical, virtual], pinned: 2, previous: 1, windows: []) == 2)
    #expect(CaptureDisplayChoice.captureRect(display: virtual.frame, application: .zero, pinned: true) == virtual.frame)
    #expect(CaptureDisplayChoice.select([physical], pinned: 2, previous: 1, windows: []) == nil)
}

@Test func remoteDesktopPresentationTracksLogicalScreenProportions() {
    let window = CGSize(width: 800, height: 500)
    let screen = CGSize(width: 1600, height: 1000)
    #expect(RemotePresentationGeometry.proportionalSize(window, sourceScreen: screen,
        destinationScreen: screen) == window)
    #expect(RemotePresentationGeometry.proportionalSize(window, sourceScreen: screen,
        destinationScreen: CGSize(width: 800, height: 500)) == CGSize(width: 400, height: 250))
    #expect(RemotePresentationGeometry.proportionalSize(window, sourceScreen: screen,
        destinationScreen: CGSize(width: 3200, height: 2000)) == CGSize(width: 1600, height: 1000))
    #expect(RemotePresentationGeometry.proportionalSize(window, sourceScreen: screen,
        destinationScreen: CGSize(width: 1200, height: 1000)) == CGSize(width: 600, height: 375))
}

@Test func remoteDesktopVirtualPresentationRestoresOriginalScreenRatio() throws {
    let original = CGSize(width: 800, height: 600)
    let screen = CGSize(width: 1600, height: 1200)
    let moved = RemotePresentationGeometry.proportionalSize(original, sourceScreen: screen,
        destinationScreen: CGSize(width: 1920, height: 1080))
    let factor = moved.width / original.width
    let reference = CGSize(width: screen.width * factor, height: screen.height * factor)
    #expect(RemotePresentationGeometry.proportionalSize(moved, sourceScreen: reference,
        destinationScreen: screen) == original)
    let surface = try CaptureSurface(identity: SurfaceIdentity(session: UUID(), epoch: 1, generation: 1, configuration: 1),
        displayID: 1, displayFrame: CGRect(x: 0, y: 0, width: 1920, height: 1080),
        contentRect: CGRect(x: 0, y: 0, width: 1920, height: 1080), pointPixelScale: 2,
        sourceHz: 60, presentationDisplaySize: reference)
    #expect(try surface.withIdentity(surface.identity).presentationDisplaySize == reference)
    let decoded = try JSONDecoder().decode(CaptureSurface.self, from: JSONEncoder().encode(surface))
    #expect(decoded.presentationDisplaySize == reference)
}
