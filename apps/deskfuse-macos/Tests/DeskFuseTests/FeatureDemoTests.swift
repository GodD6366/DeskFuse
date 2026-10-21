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
        let frame = { CrossingFrame(progress: $0, screen: self.screen, gap: 0) }
        #expect(!frame(0.3).onRemote)
        #expect(frame(0.6).onRemote)
        for p in steps {
            let current = frame(p)
            // 光标始终落在当前所属屏幕之内。
            let range = current.onRemote ? screen.width...screen.width * 2 : 0...screen.width
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

    @Test func fileArrivesBeforeTheTransferCompletes() {
        let start = FileTransferFrame(progress: 0.1)
        #expect(start.progress == 0 && start.done == 0)
        // 文件图标抵达对端之后进度条才走满，再打勾。
        #expect(FileTransferFrame(progress: 0.62).fileOpacity == 0)
        #expect(FileTransferFrame(progress: 0.72).progress == 1)
        #expect(FileTransferFrame(progress: 0.85).done == 1)
        #expect(FileTransferFrame(progress: 1).done == 0)
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
