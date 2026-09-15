import AppKit

/// Resolves and caches the icon and display name of the app that produced a
/// clipboard record, keyed by bundle id. Purely presentational; no business state.
@MainActor
final class ClipboardAppInfoCache {
    static let shared = ClipboardAppInfoCache()
    private struct Entry { let image: NSImage? }
    private var icons: [String: Entry] = [:]
    private var names: [String: String?] = [:]

    func icon(for bundleID: String?) -> NSImage? {
        guard let bundleID, !bundleID.isEmpty else { return nil }
        if let entry = icons[bundleID] { return entry.image }
        let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID)
        let image = url.map { NSWorkspace.shared.icon(forFile: $0.path) }
        icons[bundleID] = Entry(image: image)
        return image
    }

    func name(for bundleID: String?) -> String? {
        guard let bundleID, !bundleID.isEmpty else { return nil }
        if let cached = names[bundleID] { return cached }
        var value: String?
        if let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID) {
            let bundle = Bundle(url: url)
            value = (bundle?.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String)
                ?? (bundle?.object(forInfoDictionaryKey: "CFBundleName") as? String)
                ?? url.deletingPathExtension().lastPathComponent
        }
        names[bundleID] = value
        return value
    }
}
