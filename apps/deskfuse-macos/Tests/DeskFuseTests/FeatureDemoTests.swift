import CoreGraphics
import Testing
@testable import DeskFuse

struct FeatureDemoTests {
    private let screen = DemoStage.screen
    private let steps = (0...100).map { Double($0) / 100 }

    @Test func curvesStayInRangeAndPulseReturnsToRest() {
        for p in steps {
            #expect((0...1).contains(DemoCurve.smoothstep(p)))
            #expect((0...1).contains(DemoCurve.easeOut(p)))
            #expect((0...1).contains(DemoCurve.pulse(p, 0.2, 0.6)))
        }
        #expect(DemoCurve.pulse(0.1, 0.2, 0.6) == 0)
        #expect(DemoCurve.pulse(0.4, 0.2, 0.6) == 1)
        #expect(DemoCurve.pulse(0.7, 0.2, 0.6) == 0)
        #expect(DemoCurve.fade(0) == 0 && DemoCurve.fade(0.5) == 1 && DemoCurve.fade(1) == 0)
    }

    @Test func inputCursorCrossesTheSharedEdge() {
        let gap = DemoStage.gap
        let frame = { CrossingFrame(progress: $0, screen: self.screen, gap: gap) }
        #expect(!frame(0.3).onRemote)
        #expect(frame(0.6).onRemote)
        for p in steps {
            let current = frame(p)
            // 光标始终落在当前所属屏幕之内。
            let range = current.onRemote ? screen.width + gap...screen.width * 2 + gap : 0...screen.width
            #expect(range.contains(current.cursor.x))
            #expect((0...screen.height).contains(current.cursor.y))
        }
        #expect(frame(0.48).glow == 1)
        #expect(frame(0.9).glow == 0)
    }

    @Test func clipboardCopiesOnThisMacAndPastesOnTheOther() {
        #expect(ClipboardFrame(progress: 0.16).copy > 0)
        #expect(ClipboardFrame(progress: 0.16).paste == 0)
        #expect(!ClipboardFrame(progress: 0.2).onRemote)
        let landed = ClipboardFrame(progress: 0.78)
        #expect(landed.onRemote && landed.landed == 1 && landed.paste > 0)
        #expect(landed.chip.x > screen.width + DemoStage.gap)
        #expect(ClipboardFrame(progress: 0).chipOpacity == 0)
    }

    @Test func phoneSendsTheImageBeforeTheBrowserSendsTheData() {
        let start = FileTransferFrame(progress: 0.1)
        #expect(start.phoneActive && !start.browserActive)
        #expect(start.image.opacity > 0 && start.data.opacity == 0)
        // 图片先落地，此时数据还没出发。
        let imageLanded = FileTransferFrame(progress: 0.42)
        #expect(imageLanded.image.opacity == 0 && imageLanded.image.landed == 1)
        #expect(imageLanded.data.landed == 0)
        let sendingData = FileTransferFrame(progress: 0.58)
        #expect(sendingData.browserActive && !sendingData.phoneActive)
        #expect(sendingData.data.opacity > 0 && sendingData.image.landed == 1)
        let both = FileTransferFrame(progress: 0.85)
        #expect(both.image.landed == 1 && both.data.landed == 1 && both.receiving)
    }

    @Test func transferredIconsLandOnTheirTilesAndTheLoopRestartsEmpty() {
        let screen = DemoStage.screen
        let landing = FileTransferFrame(progress: 0.34)
        #expect(landing.image.position.x > screen.width + DemoStage.gap)
        let dataLanding = FileTransferFrame(progress: 0.72)
        #expect(dataLanding.data.position.x > landing.image.position.x)
        let wrapped = FileTransferFrame(progress: 1)
        #expect(wrapped.image.landed == 0 && wrapped.data.landed == 0)
        #expect(FileTransferFrame(progress: 0).image.opacity == 0)
    }

    @Test func remoteWindowMovesFromTheOtherComputerToThisMac() {
        let start = RemoteWindowFrame(progress: 0.1)
        #expect(!start.onLocal && start.center.x > screen.width + DemoStage.gap)
        let end = RemoteWindowFrame(progress: 0.8)
        #expect(end.onLocal && end.center.x < screen.width)
        #expect(end.scale > start.scale)
    }

    @Test func microphoneSignalStaysBounded() {
        for p in steps {
            let frame = MicrophoneFrame(progress: p)
            #expect(frame.waves.allSatisfy { (0..<1).contains($0) })
            #expect(frame.levels.allSatisfy { (0.25...1).contains($0) })
        }
    }

    @Test func mouseSideButtonSwitchesDesktopAndLoopsBack() {
        #expect(MouseButtonFrame(progress: 0.2).press > 0)
        #expect(MouseButtonFrame(progress: 0.6).desktop == 1)
        #expect(MouseButtonFrame(progress: 0).desktop == 0)
        #expect(MouseButtonFrame(progress: 1).desktop == 0)
    }
}
