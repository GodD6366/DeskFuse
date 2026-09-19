import AppKit
@preconcurrency import MetalKit
@preconcurrency import CoreImage

public enum VideoViewScale: String, CaseIterable, Sendable {
    case fit, actualPixels
    public var label: String { self == .fit ? "适应窗口" : "1:1 像素（⌥滚轮平移）" }
}

@MainActor public final class RemoteVideoView: MTKView {
    public var frames = LatestVideoFrame()
    public var metrics = MediaMetrics()
    public var scaleMode: VideoViewScale = .fit { didSet { offset = .zero; redraw = true; reportedSize = nil } }
    public var surfaceLayout: CaptureSurface? { didSet { if oldValue != surfaceLayout { redraw = true } } }
    public var onContentSize: ((CGSize) -> Void)?
    /// The receiver's local controls replace source-space titlebar regions.
    /// Keep it independent of the compact SwiftUI button capsule: captured
    /// indicators and browser buttons can extend below or beyond that capsule.
    public var replacesWindowControls = false {
        didSet { if oldValue != replacesWindowControls { redraw = true } }
    }
    private var windowControlsCoverRects: [CGRect] = []
    private var reportedSize: CGSize?
    private var pointerRegions: [(view: CGRect, source: CGRect)] = []
    public var onPresented: (@Sendable (SurfaceIdentity, UInt64) -> Void)?
    public var onInput: ((RemoteInputEvent, SurfaceIdentity) -> Void)?
    public var onInputDiagnostic: ((String) -> Void)?
    public var onFocus: ((Bool) -> Void)?
    public var inputEnabled = false
    public var requestControlOnNextPresentation = false { didSet { if requestControlOnNextPresentation { redraw = true } } }
    public var onAutomaticControl: (() -> Void)?
    private var videoRect = CGRect.zero
    private var presentedSurface: SurfaceIdentity?
    private var pressed = Set<UInt32>()
    private var lastPoint: CGPoint?
    private var tracking: NSTrackingArea?
    private var focusObserver: NSObjectProtocol?
    private var pointerBoundaryTimer: Timer?
    private var context: CIContext?
    private var commandQueue: MTLCommandQueue?
    private var lastSurface: SurfaceIdentity?
    private var lastSequence: UInt64 = 0
    private var lastLayout: CaptureSurface?
    private var offset = CGPoint.zero
    private var redraw = true
    private let colorSpace = CGColorSpace(name: CGColorSpace.sRGB)!

    public init() {
        let device = MTLCreateSystemDefaultDevice()
        super.init(frame: .zero, device: device)
        if let device {
            context = CIContext(mtlDevice: device, options: [.cacheIntermediates: false])
            commandQueue = device.makeCommandQueue()
        }
        colorPixelFormat = .bgra8Unorm
        framebufferOnly = false
        preferredFramesPerSecond = 120
        isPaused = false
        enableSetNeedsDisplay = false
        clearColor = MTLClearColorMake(0, 0, 0, 0)
        layer?.isOpaque = false
    }
    required init(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    public override var isOpaque: Bool { false }

    public override func layout() { super.layout(); redraw = true; reportedSize = nil }
    public override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        redraw = true
    }
    public override var acceptsFirstResponder: Bool { inputEnabled }
    public override func acceptsFirstMouse(for event: NSEvent?) -> Bool { inputEnabled }
    public override func becomeFirstResponder() -> Bool {
        onFocus?(true)
        pointerBoundaryTimer?.invalidate()
        // Tracking-area exit events can be deferred during dragging or when
        // SwiftUI overlays replace tracking areas. Check the actual pointer as
        // a fallback, including event-tracking run-loop modes.
        let timer = Timer(timeInterval: 0.05, repeats: true) { [weak self] timer in
            guard let self else { timer.invalidate(); return }
            MainActor.assumeIsolated {
                guard let window = self.window, window.firstResponder === self else {
                    self.releaseInput(); return
                }
                let point = self.convert(window.mouseLocationOutsideOfEventStream, from: nil)
                if !window.isKeyWindow || !self.containsCanvas(point) {
                    self.leaveCanvas(reason: "pointer_boundary")
                }
            }
        }
        pointerBoundaryTimer = timer
        RunLoop.main.add(timer, forMode: .common)
        return true
    }
    public override func resignFirstResponder() -> Bool { releaseInput(); return true }
    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let focusObserver { NotificationCenter.default.removeObserver(focusObserver) }
        focusObserver = nil
        if let window {
            window.acceptsMouseMovedEvents = true
            focusObserver = NotificationCenter.default.addObserver(forName: NSWindow.didResignKeyNotification,
                object: window, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { self?.releaseInput(); self?.window?.makeFirstResponder(nil) }
                }
        } else { releaseInput() }
    }
    public override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: .zero, options: [.mouseMoved, .mouseEnteredAndExited, .enabledDuringMouseDrag, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area); tracking = area
    }
    public override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(visibleRect, cursor: .arrow)
    }
    private func releaseInput() {
        pointerBoundaryTimer?.invalidate(); pointerBoundaryTimer = nil
        requestControlOnNextPresentation = false
        pressed.removeAll(); lastPoint = nil; onFocus?(false)
    }
    private func containsCanvas(_ point: CGPoint) -> Bool {
        guard videoRect.contains(point) else { return false }
        // Auxiliary panels are composited above the cover and retain input.
        if windowControlsCoverRects.contains(where: { $0.contains(point) }),
           !pointerRegions.dropFirst().contains(where: { $0.view.contains(point) }) { return false }
        guard let application = lastLayout?.applicationRect else { return true }
        let radius = min(20 * videoRect.width / application.width, min(videoRect.width, videoRect.height) / 2)
        return NSBezierPath(roundedRect: videoRect, xRadius: radius, yRadius: radius).contains(point)
    }
    public override func mouseExited(with event: NSEvent) {
        leaveCanvas(reason: "tracking_exit")
    }
    private func leaveCanvas(reason: String) {
        onInputDiagnostic?("canvas_exit_\(reason)")
        releaseInput()
        if window?.firstResponder === self { window?.makeFirstResponder(nil) }
    }
    private func pointer(_ event: NSEvent, kind: RemoteInputEvent.Kind) {
        let edge = kind == .down || kind == .up
        func trace(_ stage: String) {
            if edge { onInputDiagnostic?("pointer_\(kind.rawValue)_\(stage)") }
        }
        trace("received")
        guard inputEnabled else { trace("disabled"); return }
        guard let identity = presentedSurface, identity == lastSurface else { trace("surface_pending"); return }
        let location = convert(event.locationInWindow, from: nil)
        if window?.firstResponder === self && !containsCanvas(location) {
            leaveCanvas(reason: "pointer_event")
            return
        }
        guard frames.isCurrent(identity) else { trace("surface_pending"); return }
        let normalized = containsCanvas(location) ? pointerRegions.reversed().lazy.compactMap { region -> CGPoint? in
            guard let point = CaptureSurface.normalized(viewPoint: location, videoRect: region.view) else { return nil }
            return CGPoint(x: region.source.minX + point.x * region.source.width,
                           y: region.source.minY + point.y * region.source.height)
        }.first : nil
        // Always deliver button-up even if released over a letterbox or outside.
        guard let point = normalized ?? (kind == .up ? lastPoint : nil) else { trace("outside_video"); return }
        if kind == .down { window?.makeFirstResponder(self); pressed.insert(UInt32(event.buttonNumber)) }
        guard window?.firstResponder === self else { trace("not_focused"); return }
        if kind == .up { pressed.remove(UInt32(event.buttonNumber)) }
        lastPoint = point
        var value = RemoteInputEvent(kind: kind, point: point)
        value.flags = UInt64(event.modifierFlags.rawValue)
        // NSEvent accessors are type-specific: unsupported getters can raise
        // an Objective-C exception before onInput, which Swift cannot catch.
        if edge {
            value.button = UInt32(event.buttonNumber); value.clicks = Int64(event.clickCount)
        }
        if kind == .scroll {
            value.deltaX = Double(event.scrollingDeltaX); value.deltaY = Double(event.scrollingDeltaY)
            value.scrollPhase = event.phase.contains(.began) ? 1 : event.phase.contains(.changed) ? 2 : event.phase.contains(.ended) ? 4 : event.phase.contains(.cancelled) ? 8 : event.phase.contains(.mayBegin) ? 128 : 0
            value.momentumPhase = event.momentumPhase.contains(.began) ? 1 : event.momentumPhase.contains(.changed) ? 2 : event.momentumPhase.contains(.ended) ? 3 : 0
        }
        trace("forwarded")
        onInput?(value, identity)
    }
    public override func mouseDown(with event: NSEvent) { pointer(event, kind: .down) }
    public override func mouseUp(with event: NSEvent) { pointer(event, kind: .up) }
    public override func rightMouseDown(with event: NSEvent) { pointer(event, kind: .down) }
    public override func rightMouseUp(with event: NSEvent) { pointer(event, kind: .up) }
    public override func otherMouseDown(with event: NSEvent) { pointer(event, kind: .down) }
    public override func otherMouseUp(with event: NSEvent) { pointer(event, kind: .up) }
    public override func mouseDragged(with event: NSEvent) { pointer(event, kind: .move) }
    public override func rightMouseDragged(with event: NSEvent) { pointer(event, kind: .move) }
    public override func otherMouseDragged(with event: NSEvent) { pointer(event, kind: .move) }
    public override func mouseMoved(with event: NSEvent) { pointer(event, kind: .move) }
    public override func scrollWheel(with event: NSEvent) {
        if scaleMode == .actualPixels && event.modifierFlags.contains(.option) {
            offset.x += event.scrollingDeltaX * 2; offset.y -= event.scrollingDeltaY * 2; redraw = true
        } else { pointer(event, kind: .scroll) }
    }
    private func key(_ event: NSEvent, kind: RemoteInputEvent.Kind) {
        guard inputEnabled, window?.firstResponder === self, let identity = presentedSurface, identity == lastSurface else { return }
        guard frames.isCurrent(identity) else { return }
        // Ctrl-Option-Escape stays local; plain Escape belongs to the target app.
        if event.keyCode == 53 && event.modifierFlags.contains([.control, .option]) {
            releaseInput(); window?.makeFirstResponder(nil); return
        }
        var value = RemoteInputEvent(kind: kind); value.key = event.keyCode
        value.flags = UInt64(event.modifierFlags.rawValue)
        value.repeatKey = kind == .keyDown && event.isARepeat
        onInput?(value, identity)
    }
    public override func keyDown(with event: NSEvent) { key(event, kind: .keyDown) }
    public override func keyUp(with event: NSEvent) { key(event, kind: .keyUp) }
    public override func flagsChanged(with event: NSEvent) { key(event, kind: .flags) }
    public override func performKeyEquivalent(with event: NSEvent) -> Bool {
        guard inputEnabled, window?.firstResponder === self else { return false }
        key(event, kind: .keyDown); return true
    }

    public override func draw(_ dirtyRect: NSRect) {
        guard let presentation = frames.presentation() else {
            if lastSurface != nil, let drawable = currentDrawable, let command = commandQueue?.makeCommandBuffer(), let context {
                context.render(CIImage(color: .clear).cropped(to: CGRect(origin: .zero, size: drawableSize)),
                               to: drawable.texture, commandBuffer: command, bounds: CGRect(origin: .zero, size: drawableSize), colorSpace: colorSpace)
                command.present(drawable); command.commit()
            }
            presentedSurface = nil; lastSurface = nil; lastSequence = 0; videoRect = .zero
            pointerRegions.removeAll(); windowControlsCoverRects.removeAll()
            reportedSize = nil; lastLayout = nil; return
        }
        let frame = presentation.frame
        let changedSurface = frame.surface != lastSurface
        let fresh = changedSurface || frame.sequence != lastSequence
        let layout = presentation.layout ?? (surfaceLayout?.identity == frame.surface ? surfaceLayout : nil)
        guard fresh || redraw || layout != lastLayout,
              let context, let command = commandQueue?.makeCommandBuffer(), let drawable = currentDrawable else { return }
        let encodedSize = CGSize(width: CVPixelBufferGetWidth(frame.buffer), height: CVPixelBufferGetHeight(frame.buffer))
        let source = layout?.contentRect ?? CGRect(origin: .zero, size: encodedSize)
        let application = layout?.applicationRect ?? source
        func pixels(_ rect: CGRect) -> CGRect {
            CGRect(x: (rect.minX - source.minX) / source.width * encodedSize.width,
                   y: (source.maxY - rect.maxY) / source.height * encodedSize.height,
                   width: rect.width / source.width * encodedSize.width,
                   height: rect.height / source.height * encodedSize.height)
        }
        func normalized(_ rect: CGRect) -> CGRect {
            CGRect(x: (rect.minX - source.minX) / source.width, y: (rect.minY - source.minY) / source.height,
                   width: rect.width / source.width, height: rect.height / source.height)
        }
        let applicationPixels = pixels(application)
        let size = applicationPixels.size
        guard drawableSize.width > 0, drawableSize.height > 0, size.width > 0, size.height > 0 else { return }
        let scale = scaleMode == .fit ? min(drawableSize.width / size.width, drawableSize.height / size.height) : 1
        let scaled = CGSize(width: size.width * scale, height: size.height * scale)
        offset.x = min(max(offset.x, -max(0, scaled.width - drawableSize.width) / 2), max(0, scaled.width - drawableSize.width) / 2)
        offset.y = min(max(offset.y, -max(0, scaled.height - drawableSize.height) / 2), max(0, scaled.height - drawableSize.height) / 2)
        let origin = CGPoint(x: (drawableSize.width - scaled.width) / 2 + offset.x,
                             y: (drawableSize.height - scaled.height) / 2 + offset.y)
        func viewRect(_ rect: CGRect) -> CGRect {
            CGRect(x: rect.minX * self.bounds.width / drawableSize.width,
                   y: rect.minY * self.bounds.height / drawableSize.height,
                   width: rect.width * self.bounds.width / drawableSize.width,
                   height: rect.height * self.bounds.height / drawableSize.height)
        }
        let viewport = CGRect(origin: origin, size: scaled)
        videoRect = viewRect(viewport)
        pointerRegions = [(videoRect, normalized(application))]
        let captured = CIImage(cvPixelBuffer: frame.buffer)
        func tile(_ crop: CGRect, at destination: CGRect) -> CIImage {
            captured.cropped(to: crop)
                .transformed(by: CGAffineTransform(translationX: -crop.minX, y: -crop.minY))
                .transformed(by: CGAffineTransform(scaleX: destination.width / crop.width, y: destination.height / crop.height))
                .transformed(by: CGAffineTransform(translationX: destination.minX, y: destination.minY))
        }
        var image = tile(applicationPixels, at: viewport)
        windowControlsCoverRects.removeAll(keepingCapacity: true)
        if layout?.applicationRect != nil, replacesWindowControls {
            // Use the same source -> drawable transform as the video, including
            // fit, Retina scaling and 1:1 panning. A view-sized capsule cannot
            // reliably cover source controls. Keep the fill rectangular so its
            // rounded edges cannot expose a second set of traffic lights.
            var color = CIColor.white
            effectiveAppearance.performAsCurrentDrawingAppearance {
                if let rgb = NSColor.controlBackgroundColor.usingColorSpace(.deviceRGB) {
                    color = CIColor(red: rgb.redComponent, green: rgb.greenComponent,
                                    blue: rgb.blueComponent, alpha: 1)
                }
            }
            // Older peers omit layout metadata. An explicit empty list means
            // the controls moved outside the capture and must not get a cover.
            let covers = layout?.windowControlRects ?? [CGRect(origin: application.origin,
                size: CGSize(width: min(88, application.width), height: min(40, application.height)))]
            for cover in covers {
                let visible = cover.intersection(application)
                guard !visible.isNull, !visible.isEmpty else { continue }
                let target = CGRect(
                    x: viewport.minX + (visible.minX - application.minX) / application.width * scaled.width,
                    y: viewport.minY + (application.maxY - visible.maxY) / application.height * scaled.height,
                    width: visible.width / application.width * scaled.width,
                    height: visible.height / application.height * scaled.height)
                windowControlsCoverRects.append(viewRect(target))
                image = CIImage(color: color).cropped(to: target).composited(over: image)
            }
        }
        // Keep external candidate/popover panels inside the app viewport. Their
        // hit regions map back to the original B coordinates, never the moved UI.
        for auxiliary in layout?.auxiliaryRects ?? [] where !application.contains(auxiliary) {
            let shrink = min(1, application.width / auxiliary.width, application.height / auxiliary.height)
            let width = auxiliary.width * shrink, height = auxiliary.height * shrink
            let placed = CGRect(x: min(max(auxiliary.minX, application.minX), application.maxX - width),
                                y: min(max(auxiliary.minY, application.minY), application.maxY - height),
                                width: width, height: height)
            let target = CGRect(x: origin.x + (placed.minX - application.minX) / application.width * scaled.width,
                                y: origin.y + (application.maxY - placed.maxY) / application.height * scaled.height,
                                width: placed.width / application.width * scaled.width,
                                height: placed.height / application.height * scaled.height)
            image = tile(pixels(auxiliary), at: target).cropped(to: viewport).composited(over: image)
            pointerRegions.append((viewRect(target), normalized(auxiliary)))
        }
        let bounds = CGRect(origin: .zero, size: drawableSize)
        let transparent = CIImage(color: .clear).cropped(to: bounds)
        image = image.cropped(to: viewport)
        if layout?.applicationRect != nil,
           let mask = CIFilter(name: "CIRoundedRectangleGenerator", parameters: [
            "inputExtent": CIVector(cgRect: viewport),
            "inputRadius": min(20 * scaled.width / application.width, min(scaled.width, scaled.height) / 2),
            "inputColor": CIColor.white
           ])?.outputImage {
            // H.264 has no alpha channel. Mask just the native window corners
            // in GPU space; do not color-key black pixels from application UI.
            image = image.applyingFilter("CIBlendWithAlphaMask", parameters: [
                kCIInputBackgroundImageKey: transparent, kCIInputMaskImageKey: mask])
        }
        context.render(image.composited(over: transparent).cropped(to: bounds), to: drawable.texture,
                       commandBuffer: command, bounds: bounds, colorSpace: colorSpace)
        let metrics = metrics, callback = onPresented, surface = frame.surface, sequence = frame.sequence
        drawable.addPresentedHandler { [weak self] _ in
            if fresh { metrics.update { $0.presented += 1 }; callback?(surface, sequence) }
            Task { @MainActor [weak self] in
                guard let self, self.lastSurface == surface else { return }
                self.presentedSurface = surface
                let logicalSize = application.size
                if self.frames.isCurrent(surface), changedSurface || self.reportedSize != logicalSize {
                    self.reportedSize = logicalSize; self.onContentSize?(logicalSize)
                }
                if self.requestControlOnNextPresentation, self.window?.isKeyWindow == true,
                   let point = self.window?.mouseLocationOutsideOfEventStream,
                   self.containsCanvas(self.convert(point, from: nil)),
                   self.frames.isCurrent(surface) {
                    self.requestControlOnNextPresentation = false
                    self.window?.makeFirstResponder(self)
                    self.onAutomaticControl?()
                }
            }
        }
        command.present(drawable); command.commit()
        lastSurface = frame.surface; lastSequence = frame.sequence; lastLayout = layout; redraw = false
    }
}
