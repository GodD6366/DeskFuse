import Foundation
import SwiftUI
import Testing
import DeskFuseEngine
@testable import DeskFuse

@Suite @MainActor struct DisplayLayoutTests {
    private func displays() -> [Display] {
        [Display(id: "one", deviceId: "local", name: "Local", x: 0, y: 0, width: 1920, height: 1080,
                 scale: 1, rotation: 0, local: true, ready: true, wallpaperJpeg: nil),
         Display(id: "two", deviceId: "remote", name: "Remote", x: 1920, y: 0, width: 1920, height: 1080,
                 scale: 1, rotation: 0, local: false, ready: true, wallpaperJpeg: nil),
         Display(id: "three", deviceId: "remote", name: "Remote", x: 3840, y: 0, width: 1280, height: 720,
                 scale: 1, rotation: 0, local: false, ready: true, wallpaperJpeg: nil)]
    }

    @Test func navigationUsesKeyboardMouseSharingName() {
        #expect(Page.input.rawValue == "键鼠共享")
    }

    @Test func dragUsesStationaryCanvasRatherThanMovingTileCoordinates() {
        switch DisplayLayout.moveGesture.coordinateSpace {
        case .named(let name): #expect(name == AnyHashable(DisplayLayout.canvasSpace))
        default: Issue.record("Moving a tile must not change the gesture's coordinate space")
        }
    }

    @Test func frozenViewportDoesNotFeedSnappedPositionBackIntoTranslation() {
        let displays = displays()
        let viewport = DisplayLayoutViewport(displays: displays, size: CGSize(width: 1088, height: 640))
        let pointerMovement = CGSize(width: 64, height: 32)
        let desired = viewport.logicalTranslation(pointerMovement)
        #expect(abs(desired.width - 320) < 0.001)
        #expect(abs(desired.height - 160) < 0.001)
        let unsnapped = viewport.position(of: displays[1], translation: pointerMovement)
        let snapped = viewport.position(of: displays[1], translation: CGSize(width: 70, height: 32))
        #expect(unsnapped != snapped)
        #expect(viewport.logicalTranslation(pointerMovement) == desired)
        let resized = DisplayLayoutViewport(displays: displays, size: CGSize(width: 700, height: 400))
        #expect(resized != viewport)
        #expect(viewport.logicalTranslation(pointerMovement) == desired)
    }

    @Test func layoutConfirmationIgnoresStaleAndReadinessOnlySnapshots() {
        let before = displays(), move = CGSize(width: -1920, height: 1080)
        var heartbeat = before
        heartbeat[1].ready = false
        #expect(!DisplayLayoutConfirmation.matches(before: before, after: heartbeat, deviceID: "remote", translation: move))
        var after = before
        for index in after.indices where after[index].deviceId == "remote" {
            after[index].x += move.width; after[index].y += move.height
        }
        #expect(DisplayLayoutConfirmation.matches(before: before, after: after, deviceID: "remote", translation: move))
        // A rebased origin and different enumeration order preserve the arrangement.
        for index in after.indices { after[index].x -= 3000; after[index].y += 100 }
        after.reverse()
        #expect(DisplayLayoutConfirmation.matches(before: before, after: after, deviceID: "remote", translation: move))
    }

    @Test func layoutConfirmationRequiresAllScreensAndTheirNativeGeometry() {
        let before = displays(), move = CGSize(width: -1920, height: 1080)
        var partial = before
        partial[1].x += move.width; partial[1].y += move.height
        #expect(!DisplayLayoutConfirmation.matches(before: before, after: partial, deviceID: "remote", translation: move))
        #expect(!DisplayLayoutConfirmation.matches(before: before, after: Array(before.dropLast()), deviceID: "remote", translation: move))
        var resized = before
        resized[1].width = 2560
        #expect(!DisplayLayoutConfirmation.matches(before: before, after: resized, deviceID: "remote", translation: .zero))
    }
}
