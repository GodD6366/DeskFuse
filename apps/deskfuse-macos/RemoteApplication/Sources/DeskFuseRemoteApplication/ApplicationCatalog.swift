import AppKit

public struct RemoteApplicationInfo: Codable, Identifiable, Equatable, Sendable {
    /// Host-issued identifier. A caller cannot turn this into an arbitrary URL.
    public let id: UUID
    public let name: String
    public let bundleIdentifier: String?
    public let processID: pid_t?
    public var category: String? = nil
    public var iconPNG: Data? = nil

    public var applicationKey: String { bundleIdentifier ?? name }
}

@MainActor public final class ApplicationCatalog {
    private var urls: [UUID: URL] = [:]
    private var identifiers: [URL: UUID] = [:]
    private let icons = NSCache<NSURL, NSData>()
    public init() { icons.countLimit = 512; icons.totalCostLimit = 8 * 1024 * 1024 }

    public func applications() -> [RemoteApplicationInfo] {
        let running = NSWorkspace.shared.runningApplications.filter { $0.activationPolicy == .regular }
        var installed = Set(running.compactMap(\.bundleURL))
        let roots = [URL(fileURLWithPath: "/Applications"), URL(fileURLWithPath: "/System/Applications"),
                     FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Applications")]
        for root in roots {
            guard let enumerator = FileManager.default.enumerator(at: root, includingPropertiesForKeys: [.isDirectoryKey],
                options: [.skipsHiddenFiles, .skipsPackageDescendants]) else { continue }
            for case let url as URL in enumerator where url.pathExtension.lowercased() == "app" { installed.insert(url) }
        }
        var next: [UUID: URL] = [:]
        var iconBudget = 6 * 1024 * 1024
        let result = installed.compactMap { url -> RemoteApplicationInfo? in
            let url = url.standardizedFileURL
            guard let bundle = Bundle(url: url), url.pathExtension.lowercased() == "app" else { return nil }
            if (bundle.object(forInfoDictionaryKey: "LSUIElement") as? NSNumber)?.boolValue == true { return nil }
            if (bundle.object(forInfoDictionaryKey: "LSBackgroundOnly") as? NSNumber)?.boolValue == true { return nil }
            let id = identifiers[url] ?? UUID()
            identifiers[url] = id; next[id] = url
            let active = running.first { $0.bundleURL?.standardizedFileURL == url }
            let icon = iconBudget > 0 ? iconData(url) : nil
            iconBudget -= icon?.count ?? 0
            return RemoteApplicationInfo(id: id, name: displayName(for: url, bundle: bundle, running: active),
                bundleIdentifier: bundle.bundleIdentifier, processID: active?.processIdentifier,
                category: bundle.object(forInfoDictionaryKey: "LSApplicationCategoryType") as? String,
                iconPNG: icon)
        }.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
        urls = next
        return result
    }

    /// The catalog is built on the host, so names must follow the host's system
    /// language. Raw Info.plist keys return the development-region name
    /// (`System Settings` on a Chinese Mac); the running app's localized name and
    /// Launch Services' metadata display name both follow the user's language.
    private func displayName(for url: URL, bundle: Bundle, running: NSRunningApplication?) -> String {
        if let name = running?.localizedName, !name.isEmpty { return name }
        if let item = NSMetadataItem(url: url),
           let name = item.value(forAttribute: "kMDItemDisplayName") as? String, !name.isEmpty {
            return Self.withoutAppExtension(name)
        }
        if let name = bundle.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String { return name }
        if let name = bundle.object(forInfoDictionaryKey: "CFBundleName") as? String { return name }
        return url.deletingPathExtension().lastPathComponent
    }

    private static func withoutAppExtension(_ name: String) -> String {
        guard name.lowercased().hasSuffix(".app") else { return name }
        return String(name.dropLast(4))
    }

    private func iconData(_ url: URL) -> Data? {
        if let cached = icons.object(forKey: url as NSURL) { return cached as Data }
        guard let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 128, pixelsHigh: 128,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
              let context = NSGraphicsContext(bitmapImageRep: bitmap) else { return nil }
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context
        NSWorkspace.shared.icon(forFile: url.path).draw(in: CGRect(x: 0, y: 0, width: 128, height: 128),
            from: .zero, operation: .copy, fraction: 1)
        NSGraphicsContext.restoreGraphicsState()
        guard let data = bitmap.representation(using: .png, properties: [:]), data.count <= 48 * 1024 else { return nil }
        icons.setObject(data as NSData, forKey: url as NSURL, cost: data.count)
        return data
    }

    public func launch(_ id: UUID, activates: Bool = true) async throws -> NSRunningApplication {
        guard let url = urls[id], FileManager.default.fileExists(atPath: url.path), Bundle(url: url) != nil else {
            throw RemoteApplicationError.unsupported("应用已移除，请刷新应用列表")
        }
        let configuration = NSWorkspace.OpenConfiguration()
        // Some applications (notably System Settings and document based apps)
        // create their first on-screen window only after activation. Let
        // LaunchServices perform the initial activation; the session still
        // explicitly verifies the foreground process before arming input.
        configuration.activates = activates
        let process = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
        // Finder is always running even when it has no file-browser window.
        // Its desktop is not a movable document window. Opening a folder uses
        // LaunchServices, not Apple Events or an Automation permission prompt.
        if process.bundleIdentifier == "com.apple.finder",
           !RemoteApplicationDesktop.hasMovableWindow(processID: process.processIdentifier) {
            return try await NSWorkspace.shared.open([FileManager.default.homeDirectoryForCurrentUser],
                withApplicationAt: url, configuration: configuration)
        }
        return process
    }
}
