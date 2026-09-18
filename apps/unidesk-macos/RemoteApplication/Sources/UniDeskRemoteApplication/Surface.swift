import Foundation

public struct SurfaceIdentity: Codable, Equatable, Sendable {
    public let session: UUID
    public let epoch: UInt64
    public let generation: UInt64
    public let configuration: UInt64
    public init(session: UUID, epoch: UInt64, generation: UInt64, configuration: UInt64) {
        self.session = session; self.epoch = epoch; self.generation = generation; self.configuration = configuration
    }
}

public struct CaptureSurface: Codable, Equatable, Sendable {
    public let identity: SurfaceIdentity
    public let displayID: UInt32
    /// Quartz global logical points, including negative display origins.
    public let displayFrame: CGRect
    public let contentRect: CGRect
    public let pointPixelScale: Double
    public let sourceHz: Double?
    /// Presentation metadata. Input remains normalized to the entire capture.
    public let applicationRect: CGRect?
    public let auxiliaryRects: [CGRect]?

    public init(identity: SurfaceIdentity, displayID: UInt32, displayFrame: CGRect, contentRect: CGRect,
                pointPixelScale: Double, sourceHz: Double?, applicationRect: CGRect? = nil,
                auxiliaryRects: [CGRect]? = nil) throws {
        let values = [displayFrame.minX, displayFrame.minY, displayFrame.width, displayFrame.height,
                      contentRect.minX, contentRect.minY, contentRect.width, contentRect.height, pointPixelScale]
        guard values.allSatisfy(\.isFinite), pointPixelScale > 0, !displayFrame.isEmpty,
              !contentRect.isEmpty, displayFrame.contains(contentRect),
              contentRect.width * pointPixelScale <= 32768, contentRect.height * pointPixelScale <= 32768 else {
            throw RemoteApplicationError.invalidGeometry
        }
        self.identity = identity; self.displayID = displayID; self.displayFrame = displayFrame
        self.contentRect = contentRect; self.pointPixelScale = pointPixelScale; self.sourceHz = sourceHz
        for rect in [applicationRect].compactMap({ $0 }) + (auxiliaryRects ?? []) {
            guard [rect.minX, rect.minY, rect.width, rect.height].allSatisfy(\.isFinite),
                  !rect.isEmpty, contentRect.contains(rect) else { throw RemoteApplicationError.invalidGeometry }
        }
        self.applicationRect = applicationRect; self.auxiliaryRects = auxiliaryRects
    }

    public func withIdentity(_ identity: SurfaceIdentity) throws -> CaptureSurface {
        try CaptureSurface(identity: identity, displayID: displayID, displayFrame: displayFrame,
            contentRect: contentRect, pointPixelScale: pointPixelScale, sourceHz: sourceHz,
            applicationRect: applicationRect, auxiliaryRects: auxiliaryRects)
    }

    public var nativePixels: PixelSize {
        PixelSize(width: Int(ceil(contentRect.width * pointPixelScale)), height: Int(ceil(contentRect.height * pointPixelScale)))
    }
    public var displayLocalRect: CGRect { contentRect.offsetBy(dx: -displayFrame.minX, dy: -displayFrame.minY) }

    public func remotePoint(normalized: CGPoint) throws -> CGPoint {
        guard normalized.x.isFinite, normalized.y.isFinite, (0...1).contains(normalized.x), (0...1).contains(normalized.y) else {
            throw RemoteApplicationError.invalidGeometry
        }
        return CGPoint(x: contentRect.minX + normalized.x * contentRect.width,
                       y: contentRect.minY + normalized.y * contentRect.height)
    }

    /// A Cocoa view point (bottom-left origin) -> video position (top-left origin).
    /// Reject letterbox clicks; never clamp them onto an unrelated remote window.
    public static func normalized(viewPoint: CGPoint, videoRect: CGRect) -> CGPoint? {
        guard !videoRect.isEmpty, videoRect.contains(viewPoint) else { return nil }
        return CGPoint(x: (viewPoint.x - videoRect.minX) / videoRect.width,
                       y: 1 - (viewPoint.y - videoRect.minY) / videoRect.height)
    }
}

public struct CropTracker: Sendable {
    public private(set) var current: CGRect?
    public init() {}
    public mutating func update(windows: [CGRect], display: CGRect, gestureActive: Bool) -> CGRect? {
        if gestureActive, let current { return current }
        let visible = windows.map { $0.intersection(display) }.filter { !$0.isNull && !$0.isEmpty }
        guard var union = visible.first else { current = nil; return nil }
        for rect in visible.dropFirst() { union = union.union(rect) }
        // Integral logical bounds enclose the complete windows, including sheets.
        current = union.integral.intersection(display)
        return current
    }
}

/// Input may only be armed after the first frame of the exact new configuration
/// has actually been presented. Callers must release native held input on revoke.
public struct SurfaceGate: Sendable {
    public private(set) var expected: SurfaceIdentity?
    public private(set) var presented = false
    private var lastSequence: UInt64 = 0
    public init() {}
    public mutating func begin(_ identity: SurfaceIdentity) { expected = identity; presented = false; lastSequence = 0 }
    @discardableResult public mutating func acknowledge(_ identity: SurfaceIdentity) -> Bool {
        guard expected == identity else { return false }
        presented = true; return true
    }
    public mutating func accept(_ identity: SurfaceIdentity, sequence: UInt64) -> Bool {
        guard presented, expected == identity, sequence > lastSequence else { return false }
        lastSequence = sequence; return true
    }
    public mutating func revoke() { expected = nil; presented = false; lastSequence = 0 }
}
