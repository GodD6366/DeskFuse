// swift-tools-version: 6.0
import PackageDescription
import Foundation
let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
let libraryDirectory = ProcessInfo.processInfo.environment["UNIDESK_RUST_LIBRARY_DIR"] ?? root.appendingPathComponent("target/debug").path
let package = Package(
    name: "uniDesk", platforms: [.macOS(.v14)],
    products: [.executable(name: "unidesk-app", targets: ["UniDesk"])],
    targets: [
        .systemLibrary(name: "UniDeskEngineFFI", path: "Sources/UniDeskEngineFFI"),
        .target(name: "UniDeskEngine", dependencies: ["UniDeskEngineFFI"],
                linkerSettings: [.unsafeFlags([libraryDirectory + "/libunidesk_ffi.a"])]),
        .executableTarget(name: "UniDesk", dependencies: ["UniDeskEngine"],
            linkerSettings: [.linkedFramework("AppKit"), .linkedFramework("OSAKit"), .linkedFramework("CoreGraphics"), .linkedFramework("CoreFoundation"),
                .linkedFramework("Security"), .linkedFramework("SystemConfiguration"), .linkedFramework("CoreWLAN"), .linkedFramework("CoreLocation"),
                .linkedLibrary("sqlite3"), .linkedLibrary("resolv"), .linkedLibrary("iconv")])
    ]
)
