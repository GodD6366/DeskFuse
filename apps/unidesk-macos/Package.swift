// swift-tools-version: 6.0
import PackageDescription
import Foundation
let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
let libraryDirectory = ProcessInfo.processInfo.environment["UNIDESK_RUST_LIBRARY_DIR"] ?? root.appendingPathComponent("target/debug").path
let freerdpLibDirectory = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("RemoteApplication/Vendor/FreeRDP/lib").path
let homebrewPrefix = ProcessInfo.processInfo.environment["HOMEBREW_PREFIX"] ?? "/opt/homebrew"
let systemLibDirectories = [homebrewPrefix + "/lib", homebrewPrefix + "/opt/openssl@3/lib", homebrewPrefix + "/opt/json-c/lib", homebrewPrefix + "/opt/libusb/lib"]
let package = Package(
    name: "uniDesk", platforms: [.macOS(.v14)],
    products: [.executable(name: "unidesk-app", targets: ["UniDesk"])],
    dependencies: [
        .package(url: "https://github.com/jaywcjlove/PermissionFlow.git", from: "2.11.2")
    ],
    targets: [
        .systemLibrary(name: "CUniDeskRdp", path: "RemoteApplication/Vendor/FreeRDP"),
        .target(name: "UniDeskRemoteApplication", path: "RemoteApplication/Sources/UniDeskRemoteApplication",
            dependencies: ["CUniDeskRdp"],
            exclude: ["RemoteTransport.swift", "RemoteTLSIdentity.swift", "RdpBridge"],
            linkerSettings: [
                .unsafeFlags(["-L", freerdpLibDirectory] + systemLibDirectories.flatMap { ["-L", $0] }),
                .linkedLibrary("ssl"), .linkedLibrary("crypto"),
                .linkedLibrary("usb-1.0"), .linkedLibrary("json-c"),
                .linkedFramework("Foundation"), .linkedFramework("CoreServices"), .linkedFramework("Security"),
                .linkedFramework("ScreenCaptureKit"), .linkedFramework("VideoToolbox"),
                .linkedFramework("MetalKit"), .linkedFramework("CoreImage"),
            ]),
        .systemLibrary(name: "UniDeskEngineFFI", path: "Sources/UniDeskEngineFFI"),
        .target(name: "UniDeskEngine", dependencies: ["UniDeskEngineFFI"],
                linkerSettings: [.unsafeFlags([libraryDirectory + "/libunidesk_ffi.a"])]),
        .target(name: "UniDeskHost", dependencies: [], path: "Host"),
        .executableTarget(name: "UniDesk", dependencies: ["UniDeskEngine", "UniDeskHost", "UniDeskRemoteApplication",
                .product(name: "PermissionFlow", package: "PermissionFlow"),
                .product(name: "PermissionFlowScreenRecordingStatus", package: "PermissionFlow")],
            linkerSettings: [.linkedFramework("AppKit"), .linkedFramework("OSAKit"), .linkedFramework("CoreGraphics"), .linkedFramework("CoreFoundation"),
                .linkedFramework("Security"), .linkedFramework("SystemConfiguration"), .linkedFramework("CoreWLAN"), .linkedFramework("CoreLocation"),
                .linkedLibrary("sqlite3"), .linkedLibrary("resolv"), .linkedLibrary("iconv")])
    ]
)
