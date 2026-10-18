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

struct CaptureDisplayChoice {
    let id: UInt32
    let frame: CGRect
    static func captureRect(display: CGRect, application: CGRect, pinned: Bool) -> CGRect {
        // A dedicated display is a stable input coordinate space. Activation,
        // Dock/menu changes and dialogs update layout, not the native lease.
        pinned ? display : application.intersection(display)
    }
    static func select(_ displays: [Self], pinned: UInt32?, previous: UInt32?, windows: [CGRect]) -> UInt32? {
        // Explicit assignments are hard boundaries; a lost virtual screen must
        // never silently select a physical desktop instead.
        if let pinned { return displays.first { $0.id == pinned }?.id }
        if let previous, displays.contains(where: { $0.id == previous }) { return previous }
        func area(_ display: Self) -> CGFloat {
            windows.reduce(0) { total, window in
                let intersection = window.intersection(display.frame)
                return total + (intersection.isNull ? 0 : intersection.width * intersection.height)
            }
        }
        return displays.max { area($0) < area($1) }?.id
    }
}

/// A short enumeration gap is recoverable; a persistently absent window is not.
/// Uses monotonic time and resets only after a complete successful refresh.
public struct CaptureWindowAvailabilityGrace {
    public static let duration: TimeInterval = 3
    private var missingSince: TimeInterval?
    public var isRecovering: Bool { missingSince != nil }
    public init() {}
    public mutating func allowsRetry(at now: TimeInterval) -> Bool {
        if missingSince == nil { missingSince = now }
        return now - missingSince! < Self.duration
    }
    public mutating func reset() { missingSince = nil }
}

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
    private let sampleQueue = DispatchQueue(label: "DeskFuse.remote.capture", qos: .userInteractive)
    private var crop = CropTracker()
    private var currentDisplay: UInt32?
    private var filterApplications: [pid_t] = []
    public private(set) var sourceStage = "idle"
    public init() {}
    public static var hasScreenRecordingPermission: Bool { CGPreflightScreenCaptureAccess() }
    @discardableResult public static func requestScreenRecordingPermission() -> Bool { CGRequestScreenCaptureAccess() }

    public func source(processID: pid_t, displayID: UInt32?, mode: CaptureViewMode,
                       identity: SurfaceIdentity, gestureActive: Bool = false) async throws -> Source {
        guard Self.hasScreenRecordingPermission else {
            throw RemoteApplicationError.permission("请在被控 Mac 的系统设置中允许 DeskFuse 录制屏幕")
        }
        sourceStage = "shareable_content"
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
        sourceStage = "sck_application"
        guard let application = content.applications.first(where: { $0.processID == processID }) else { throw RemoteApplicationError.noWindow }
        let ownedWindows = content.windows.filter { $0.owningApplication?.processID == processID }
        let windows = ownedWindows.filter { $0.isOnScreen && !$0.frame.isEmpty }
        let selectedDisplay = CaptureDisplayChoice.select(content.displays.map { .init(id: $0.displayID, frame: $0.frame) },
            pinned: displayID, previous: gestureActive ? currentDisplay : nil, windows: windows.map(\.frame))
        sourceStage = "sck_assigned_display"
        guard let display = content.displays.first(where: { $0.displayID == selectedDisplay }) else {
            throw RemoteApplicationError.noWindow
        }
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
            sourceStage = "sck_window_crop"
            let hasVisibleWindow = baseWindows.contains {
                let region = $0.frame.intersection(display.frame)
                return !region.isNull && !region.isEmpty
            }
            let visibleUnion = crop.update(windows: baseWindows.map(\.frame), display: display.frame,
                gestureActive: gestureActive && (displayID == nil || hasVisibleWindow))
            // The assigned screen remains the capture boundary even when SCK
            // briefly omits Finder's document window. Never substitute a
            // physical display or invent an application/input region.
            guard let union = visibleUnion ?? (displayID != nil ? display.frame : nil) else {
                throw RemoteApplicationError.noWindow
            }
            applicationRect = visibleUnion
            if candidateBase != union || candidateDisplay != display.displayID {
                candidateBase = union; candidateExtent = union; candidateDisplay = display.displayID
            }
            if !gestureActive {
                for window in windows + auxiliaryWindows {
                    candidateExtent = candidateExtent.union(window.frame.intersection(display.frame))
                }
            }
            rect = CaptureDisplayChoice.captureRect(display: display.frame, application: candidateExtent, pinned: displayID != nil)
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
        sourceStage = mode == .application && applicationRect == nil ? "sck_window_crop_missing" : "ready"
        let note = "windowRegion=\(applicationRect == nil ? "missing" : "visible") bundle=\(application.bundleIdentifier) owned=\(ownedWindows.count) onScreen=\(windows.count) chromium=\(chromiumMatched) displayFilter=\(usesDisplayFilter) display=\(display.displayID) pinned=\(displayID != nil)"
        return Source(filter: filter, surface: try CaptureSurface(identity: identity, displayID: display.displayID,
            displayFrame: display.frame, contentRect: rect, pointPixelScale: scale, sourceHz: hz,
            applicationRect: applicationRect, auxiliaryRects: auxiliaryRects),
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
