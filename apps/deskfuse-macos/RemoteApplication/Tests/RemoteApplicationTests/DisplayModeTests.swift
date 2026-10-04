import Foundation
import Testing
@testable import DeskFuseRemoteApplication

@Test(arguments: [RemoteApplicationDisplayMode.window, .virtualDisplay])
func displayModeLaunchWireRoundTrip(mode: RemoteApplicationDisplayMode) throws {
    var message = RemoteMessage(.launch)
    message.application = UUID()
    message.displayMode = mode
    let decoded = try RemoteWire.decode(Data(RemoteWire.encode(message).dropFirst(4)))
    #expect(decoded.application == message.application)
    #expect(decoded.displayMode == mode)
}

@Test func displayModeAbsentOnOlderLaunchUsesOrdinaryWindows() throws {
    let message = RemoteMessage(.launch)
    let decoded = try RemoteWire.decode(Data(RemoteWire.encode(message).dropFirst(4)))
    #expect(decoded.displayMode == nil)
    #expect((decoded.displayMode ?? .window) == .window)
}

@Test func displayModeOffNeedsNoVirtualDisplayAndDoesNotPinCapture() throws {
    #expect(try RemoteApplicationDisplayMode.window.captureDisplayID(assigned: nil) == nil)
    #expect(try RemoteApplicationDisplayMode.window.captureDisplayID(assigned: 123) == nil)
}

@Test func displayModeOnNeverFallsBackAfterAllocationLoss() throws {
    #expect(try RemoteApplicationDisplayMode.virtualDisplay.captureDisplayID(assigned: 123) == 123)
    #expect(throws: (any Error).self) {
        try RemoteApplicationDisplayMode.virtualDisplay.captureDisplayID(assigned: nil)
    }
    #expect(throws: (any Error).self) {
        try RemoteApplicationDisplayMode.virtualDisplay.captureDisplayID(assigned: 0)
    }
}
