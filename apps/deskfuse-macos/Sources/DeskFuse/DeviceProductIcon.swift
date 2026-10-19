import AppKit
import Darwin
import SwiftUI

/// Renders the same system supplied Mac artwork used by Finder and BlueSync.
/// The icon files are part of macOS, so no product artwork is bundled or
/// maintained by DeskFuse. A remote peer may only provide a display name today;
/// in that case the resolver uses the model words in that name and falls back
/// to the generic system sidebar icon.
struct DeviceProductIconView: View {
    let displayName: String
    var modelIdentifier: String? = nil
    var size: CGFloat = 42
    var statusTint: Color? = nil

    private var icon: NSImage? {
        DeviceProductIconResolver.image(for: displayName, modelIdentifier: modelIdentifier)
    }

    var body: some View {
        ZStack(alignment: .bottomTrailing) {
            if let icon {
                Image(nsImage: icon)
                    .resizable()
                    .interpolation(.high)
                    .scaledToFit()
                    .frame(width: size, height: size)
            } else {
                Image(systemName: DeviceProductIconResolver.fallbackSymbol(for: displayName))
                    .font(DFType.scaledIcon(size * 0.66, weight: .light))
                    .foregroundStyle(.secondary)
                    .frame(width: size, height: size)
            }

            if let statusTint {
                Circle()
                    .fill(statusTint)
                    .frame(width: max(7, size * 0.2), height: max(7, size * 0.2))
                    .overlay {
                        Circle().stroke(Color(nsColor: .windowBackgroundColor), lineWidth: 1.5)
                    }
                    .offset(x: 1, y: 1)
            }
        }
        .frame(width: size, height: size)
        .accessibilityLabel("\(displayName) 设备")
    }
}

enum DeviceProductIconResolver {
    private static let coreTypesResources = URL(filePath: "/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources")

    static func image(for displayName: String, modelIdentifier: String? = nil) -> NSImage? {
        guard let url = iconURL(for: displayName, modelIdentifier: modelIdentifier),
              let image = NSImage(contentsOf: url)
        else { return nil }
        image.isTemplate = false
        return image
    }

    static func fallbackSymbol(for displayName: String) -> String {
        let value = normalized(displayName)
        if value.contains("iphone") || value.contains("ipad") || value.contains("手机") { return "iphone" }
        if value.contains("windows") || value.contains("desktop") || value.contains("pc") { return "desktopcomputer" }
        return "laptopcomputer"
    }

    static var localModelIdentifier: String? {
        var size = 0
        guard sysctlbyname("hw.model", nil, &size, nil, 0) == 0, size > 0 else { return nil }
        var buffer = [CChar](repeating: 0, count: size)
        guard sysctlbyname("hw.model", &buffer, &size, nil, 0) == 0 else { return nil }
        // `String(cString:)` is deprecated for the CChar alias on the current
        // SDK. Keep the sysctl result lossless while avoiding a forced UTF-8
        // reinterpretation of the trailing NUL byte.
        let bytes = buffer.prefix { $0 != 0 }.map(UInt8.init)
        return String(decoding: bytes, as: UTF8.self)
    }

    private static func iconURL(for displayName: String, modelIdentifier: String?) -> URL? {
        if let modelIdentifier,
           let url = SystemDeviceIconCatalog.shared.iconURL(for: modelIdentifier) {
            return url
        }

        let value = normalized([displayName, modelIdentifier].compactMap { $0 }.joined(separator: " "))
        let fileName: String?
        if value.contains("macbook air") || value.contains("macbookair") {
            fileName = "com.apple.macbookair-13-2022-silver.icns"
        } else if value.contains("macbook pro") || value.contains("macbookpro") || value.contains("mbp") {
            fileName = "com.apple.macbookpro-14-2021-silver.icns"
        } else if value.contains("macbook") || value.contains("mb ") {
            fileName = "com.apple.macbook-retina-silver.icns"
        } else if value.contains("mac mini") || value.contains("macmini") || value.contains("mac mini") || value.contains("mac-mini") || value.contains("mini") {
            fileName = "com.apple.macmini-2020.icns"
        } else if value.contains("mac studio") || value.contains("macstudio") {
            fileName = "com.apple.macstudio.icns"
        } else if value.contains("imac pro") || value.contains("imacpro") {
            fileName = "com.apple.imacpro-2017.icns"
        } else if value.contains("imac") {
            fileName = "com.apple.imac-2021-silver.icns"
        } else if value.contains("mac pro") || value.contains("macpro") {
            fileName = "com.apple.macpro-2019.icns"
        } else {
            fileName = nil
        }

        guard let fileName else { return nil }
        let url = coreTypesResources.appendingPathComponent(fileName)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    private static func normalized(_ value: String) -> String {
        value
            .lowercased()
            .replacingOccurrences(of: "-", with: " ")
            .replacingOccurrences(of: "_", with: " ")
            .replacingOccurrences(of: ".", with: " ")
    }
}

/// CoreTypes keeps model-code declarations beside the corresponding icon. This
/// mirrors BlueSync's resolver and lets the local Mac use the exact artwork
/// for its `hw.model`, including color variants.
private struct SystemDeviceIconCatalog {
    static let shared = SystemDeviceIconCatalog()

    private static let coreTypes = URL(filePath: "/System/Library/CoreServices/CoreTypes.bundle/Contents")
    private let exact: [String: URL]
    private let colorFallback: [String: URL]

    init() {
        let declarations = Self.loadDeclarations()
        var exact: [String: URL] = [:]
        var colorFallback: [String: URL] = [:]
        for declaration in declarations.values where !declaration.modelCodes.isEmpty {
            var visited = Set<String>()
            guard let icon = Self.resolveIcon(for: declaration.identifier, declarations: declarations, visited: &visited) else { continue }
            for modelCode in declaration.modelCodes {
                let normalized = modelCode.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
                if let base = Self.baseModel(for: normalized) {
                    colorFallback[base] = colorFallback[base] ?? icon
                } else {
                    exact[normalized] = icon
                }
            }
        }
        self.exact = exact
        self.colorFallback = colorFallback
    }

    func iconURL(for modelIdentifier: String) -> URL? {
        let normalized = modelIdentifier.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if let url = exact[normalized] { return url }
        if let base = Self.baseModel(for: normalized), let url = colorFallback[base] { return url }
        return colorFallback[normalized]
    }

    private static func loadDeclarations() -> [String: DeviceIconDeclaration] {
        var result: [String: DeviceIconDeclaration] = [:]
        for plistURL in infoPlistURLs() {
            guard let plist = NSDictionary(contentsOf: plistURL) as? [String: Any] else { continue }
            let resources = plistURL.deletingLastPathComponent().lastPathComponent == "Contents"
                ? plistURL.deletingLastPathComponent().appendingPathComponent("Resources")
                : plistURL.deletingLastPathComponent()
            for key in ["UTExportedTypeDeclarations", "UTImportedTypeDeclarations"] {
                guard let items = plist[key] as? [[String: Any]] else { continue }
                for item in items {
                    guard let identifier = item["UTTypeIdentifier"] as? String else { continue }
                    let declaration = DeviceIconDeclaration(
                        identifier: identifier,
                        parents: strings(item["UTTypeConformsTo"]),
                        iconFileName: (item["UTTypeIcons"] as? [String: Any])?["UTTypeIconFile"] as? String,
                        modelCodes: strings((item["UTTypeTagSpecification"] as? [String: Any])?["com.apple.device-model-code"]),
                        resources: resources
                    )
                    result[identifier] = declaration.merging(result[identifier])
                }
            }
        }
        return result
    }

    private static func infoPlistURLs() -> [URL] {
        var urls = [coreTypes.appendingPathComponent("Info.plist")]
        let library = coreTypes.appendingPathComponent("Library")
        guard let enumerator = FileManager.default.enumerator(at: library, includingPropertiesForKeys: [.isRegularFileKey], options: [.skipsHiddenFiles]) else { return urls }
        for case let url as URL in enumerator where url.lastPathComponent == "Info.plist" { urls.append(url) }
        return urls
    }

    private static func resolveIcon(for identifier: String, declarations: [String: DeviceIconDeclaration], visited: inout Set<String>) -> URL? {
        guard visited.insert(identifier).inserted, let declaration = declarations[identifier] else { return nil }
        if let fileName = declaration.iconFileName, let url = existingIcon(fileName, resources: declaration.resources) { return url }
        for parent in declaration.parents {
            if let url = resolveIcon(for: parent, declarations: declarations, visited: &visited) { return url }
        }
        return nil
    }

    private static func existingIcon(_ fileName: String, resources: URL) -> URL? {
        let candidates = fileName.hasSuffix(".icns") ? [fileName] : [fileName, "\(fileName).icns"]
        for root in [resources, coreTypes.appendingPathComponent("Resources")] {
            for candidate in candidates {
                let url = root.appendingPathComponent(candidate)
                if FileManager.default.fileExists(atPath: url.path) { return url }
            }
        }
        return nil
    }

    private static func strings(_ value: Any?) -> [String] {
        if let string = value as? String { return [string] }
        return (value as? [String]) ?? []
    }

    private static func baseModel(for value: String) -> String? {
        guard let range = value.range(of: "@ecolor=") else { return nil }
        return String(value[..<range.lowerBound])
    }
}

private struct DeviceIconDeclaration {
    let identifier: String
    let parents: [String]
    let iconFileName: String?
    let modelCodes: [String]
    let resources: URL

    func merging(_ other: DeviceIconDeclaration?) -> DeviceIconDeclaration {
        guard let other else { return self }
        return DeviceIconDeclaration(
            identifier: identifier,
            parents: parents.isEmpty ? other.parents : parents,
            iconFileName: iconFileName ?? other.iconFileName,
            modelCodes: Array(Set(modelCodes + other.modelCodes)),
            resources: iconFileName == nil && other.iconFileName != nil ? other.resources : resources
        )
    }
}
