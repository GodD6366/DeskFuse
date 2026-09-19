import AppKit
import Carbon
@preconcurrency import ScreenCaptureKit
@preconcurrency import CoreMedia

private final class CaptureOutput: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    let surface: SurfaceIdentity
    let metrics: MediaMetrics
    let sink: @Sendable (CapturedVideoFrame) -> Void
    let failed: @Sendable (String) -> Void
    init(surface: SurfaceIdentity, metrics: MediaMetrics, sink: @escaping @Sendable (CapturedVideoFrame) -> Void,
         failed: @escaping @Sendable (String) -> Void) {
        self.surface = surface; self.metrics = metrics; self.sink = sink; self.failed = failed
    }
    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, sampleBuffer.isValid,
              let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let raw = attachments.first?[.status] as? Int, let status = SCFrameStatus(rawValue: raw) else {
            metrics.update { $0.capturedOther += 1 }
            return
        }
        guard status == .complete, let image = CMSampleBufferGetImageBuffer(sampleBuffer) else {
            switch status {
            case .idle: metrics.update { $0.capturedIdle += 1 }
            case .blank: metrics.update { $0.capturedBlank += 1 }
            case .started: metrics.update { $0.capturedStarted += 1 }
            default: metrics.update { $0.capturedOther += 1 }
            }
            return
        }
        metrics.update { $0.captured += 1 }
        sink(CapturedVideoFrame(surface: surface, buffer: image, timestamp: CMSampleBufferGetPresentationTimeStamp(sampleBuffer)))
    }
    func stream(_ stream: SCStream, didStopWithError error: any Error) { failed(error.localizedDescription) }
}

public enum CaptureViewMode: String, Sendable { case application, desktop }

@available(macOS 14.2, *)
@MainActor public final class RemoteAppCapture {
    public struct Source {
        public let filter: SCContentFilter
        public let surface: CaptureSurface
        public let applicationIDs: [pid_t]
        /// Host-side capture provenance for diagnostics; never crosses the wire.
        public let note: String
    }
    private var stream: SCStream?
    private var output: CaptureOutput?
    private let sampleQueue = DispatchQueue(label: "uniDesk.remote.capture", qos: .userInteractive)
    private var crop = CropTracker()
    private var currentDisplay: UInt32?
    private var filterApplications: [pid_t] = []
    public init() {}
    public static var hasScreenRecordingPermission: Bool { CGPreflightScreenCaptureAccess() }
    @discardableResult public static func requestScreenRecordingPermission() -> Bool { CGRequestScreenCaptureAccess() }

    public func source(processID: pid_t, displayID: UInt32?, mode: CaptureViewMode,
                       identity: SurfaceIdentity, gestureActive: Bool = false) async throws -> Source {
        guard Self.hasScreenRecordingPermission else {
            throw RemoteApplicationError.permission("请在被控 Mac 的系统设置中允许 uniDesk 录制屏幕")
        }
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
        guard let application = content.applications.first(where: { $0.processID == processID }) else { throw RemoteApplicationError.noWindow }
        let ownedWindows = content.windows.filter { $0.owningApplication?.processID == processID }
        let windows = ownedWindows.filter { $0.isOnScreen && !$0.frame.isEmpty }
        let pinnedDisplay = displayID ?? (gestureActive ? currentDisplay : nil)
        let display = pinnedDisplay.flatMap { id in content.displays.first { $0.displayID == id } }
            ?? content.displays.max { a, b in
                func area(_ display: SCDisplay) -> CGFloat {
                    windows.reduce(0) { total, window in
                        let intersection = window.frame.intersection(display.frame)
                        return total + (intersection.isNull ? 0 : intersection.width * intersection.height)
                    }
                }
                return area(a) < area(b)
            }
        guard let display else { throw RemoteApplicationError.noWindow }
        currentDisplay = display.displayID
        // Candidate panels are owned by input-method processes, not the editor.
        // Include those applications so new candidate windows appear immediately
        // without replacing the video stream on every composition update.
        let activeInputBundle: String? = {
            guard let source = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue(),
                  let value = TISGetInputSourceProperty(source, kTISPropertyBundleID) else { return nil }
            return Unmanaged<CFString>.fromOpaque(value).takeUnretainedValue() as String
        }()
        let isForeground = NSWorkspace.shared.frontmostApplication?.processIdentifier == processID
        let inputMethods = content.applications.filter { app in
            guard isForeground else { return false }
            guard app.processID != processID else { return false }
            let bundle = app.bundleIdentifier
            let path = NSRunningApplication(processIdentifier: app.processID)?.bundleURL?.path ?? ""
            return bundle == activeInputBundle || bundle.hasPrefix("com.apple.inputmethod.")
                || ["com.apple.TextInputUI", "com.apple.TextInputMenuAgent", "com.apple.CharacterPaletteIM"].contains(bundle)
                || path.contains("/Input Methods/")
        }
        let included = [application] + inputMethods
        let inputMethodIDs = Set(inputMethods.map(\.processID))
        let auxiliaryWindows = content.windows.filter {
            $0.owningApplication.map { inputMethodIDs.contains($0.processID) } == true
                && $0.isOnScreen && !$0.frame.isEmpty && $0.frame.intersects(display.frame)
        }
        let filter: SCContentFilter
        let rect: CGRect
        var applicationRect: CGRect?
        var auxiliaryRects: [CGRect] = []
        var windowControlRects: [CGRect] = []
        var usesDisplayFilter = mode == .desktop
        var chromiumMatched = false
        if mode == .desktop {
            filter = SCContentFilter(display: display, excludingWindows: [])
            rect = display.frame
        } else {
            // Chromium renders the page through separate GPU/renderer surfaces.
            // Both application and window filters can then return only the
            // transparent desktop-independent layer. Capture the display for
            // Chromium and keep the stream's sourceRect limited to the union
            // of the browser windows calculated below.
            let chromiumBundleIDs: Set<String> = [
                "com.google.Chrome", "com.google.Chrome.canary", "org.chromium.Chromium",
                "com.microsoft.edgemac", "com.brave.Browser", "com.vivaldi.Vivaldi"
            ]
            chromiumMatched = chromiumBundleIDs.contains(application.bundleIdentifier) && !windows.isEmpty
            if chromiumMatched {
                filter = SCContentFilter(display: display, excludingWindows: [])
                usesDisplayFilter = true
            } else {
                filter = SCContentFilter(display: display, including: included, exceptingWindows: [])
            }
            // Retain room already used by a candidate panel while typing. This
            // prevents every show/hide from changing coordinates and the lease.
            let normalWindows = windows.filter { $0.windowLayer == 0 }
            let baseWindows = normalWindows.isEmpty ? windows : normalWindows
            guard let union = crop.update(windows: baseWindows.map(\.frame), display: display.frame, gestureActive: gestureActive) else {
                throw RemoteApplicationError.noWindow
            }
            applicationRect = union
            if candidateBase != union || candidateDisplay != display.displayID {
                candidateBase = union; candidateExtent = union; candidateDisplay = display.displayID
            }
            if !gestureActive {
                for window in windows + auxiliaryWindows {
                    candidateExtent = candidateExtent.union(window.frame.intersection(display.frame))
                }
            }
            rect = candidateExtent.intersection(display.frame)
            // Derive controls from live window frames, never the frozen crop's
            // origin. Clip after positioning, so off-screen controls stay gone.
            windowControlRects = baseWindows.compactMap { window in
                let cover = CGRect(origin: window.frame.origin,
                    size: CGSize(width: min(88, window.frame.width), height: min(40, window.frame.height)))
                    .intersection(rect)
                return cover.isNull || cover.isEmpty ? nil : cover
            }
            let baseWindowIDs = Set(baseWindows.map(\.windowID))
            // A moving main window is still part of the base video. Treating
            // its overflow as an auxiliary panel composites it a second time.
            auxiliaryRects = (windows.filter { !baseWindowIDs.contains($0.windowID) } + auxiliaryWindows).compactMap { window in
                let visible = window.frame.intersection(rect)
                guard !visible.isNull, !visible.isEmpty, !union.contains(visible) else { return nil }
                return visible
            }
        }
        filter.includeMenuBar = mode == .desktop
        let refresh = CGDisplayCopyDisplayMode(display.displayID)?.refreshRate ?? 0
        let screen = NSScreen.screens.first { ($0.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber)?.uint32Value == display.displayID }
        let maximum = screen?.maximumFramesPerSecond ?? 0
        let hz: Double? = refresh > 0 ? refresh : (maximum > 0 ? Double(maximum) : nil)
        let scale = Double(filter.pointPixelScale)
        let note = "bundle=\(application.bundleIdentifier) owned=\(ownedWindows.count) onScreen=\(windows.count) chromium=\(chromiumMatched) displayFilter=\(usesDisplayFilter)"
        return Source(filter: filter, surface: try CaptureSurface(identity: identity, displayID: display.displayID,
            displayFrame: display.frame, contentRect: rect, pointPixelScale: scale, sourceHz: hz,
            applicationRect: applicationRect, auxiliaryRects: auxiliaryRects, windowControlRects: windowControlRects),
            applicationIDs: mode == .desktop ? [] : included.map(\.processID).sorted(), note: note)
    }

    private var candidateBase: CGRect?
    private var candidateExtent = CGRect.zero
    private var candidateDisplay: UInt32?

    public func updateFilter(_ source: Source) async throws {
        guard filterApplications != source.applicationIDs, let stream else { return }
        try await stream.updateContentFilter(source.filter)
        filterApplications = source.applicationIDs
    }

    public func start(source: Source, profile: VideoProfile, metrics: MediaMetrics,
                      sink: @escaping @Sendable (CapturedVideoFrame) -> Void,
                      failed: @escaping @Sendable (String) -> Void) async throws {
        await stop()
        let configuration = SCStreamConfiguration()
        configuration.width = profile.pixels.width; configuration.height = profile.pixels.height
        configuration.sourceRect = source.surface.displayLocalRect
        // The receiver maps the entire encoded buffer to sourceRect. Do not
        // let ScreenCaptureKit insert its own aspect-fit margins (including
        // when an IME panel expands the source or H.264 rounds to even pixels).
        configuration.destinationRect = CGRect(x: 0, y: 0, width: profile.pixels.width, height: profile.pixels.height)
        configuration.preservesAspectRatio = false
        configuration.minimumFrameInterval = CMTime(seconds: 1 / profile.framesPerSecond, preferredTimescale: 1_000_000)
        configuration.queueDepth = 3
        configuration.pixelFormat = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        configuration.captureResolution = .best
        configuration.includeChildWindows = true
        configuration.ignoreShadowsDisplay = true
        // A uses its normal local pointer; baking B's cursor into the video
        // shows a second, delayed pointer and prevents truly idle frames.
        configuration.showsCursor = false
        configuration.capturesAudio = false
        configuration.colorSpaceName = CGColorSpace.sRGB
        configuration.colorMatrix = CGDisplayStream.yCbCrMatrix_ITU_R_709_2
        let output = CaptureOutput(surface: source.surface.identity, metrics: metrics, sink: sink, failed: failed)
        let stream = SCStream(filter: source.filter, configuration: configuration, delegate: output)
        try stream.addStreamOutput(output, type: .screen, sampleHandlerQueue: sampleQueue)
        self.output = output; self.stream = stream
        filterApplications = source.applicationIDs
        do { try await stream.startCapture() }
        catch { self.stream = nil; self.output = nil; throw error }
    }
    public func stop() async {
        let old = stream; stream = nil
        try? await old?.stopCapture()
        output = nil
    }
}
