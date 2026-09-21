// swift-tools-version: 6.0
import PackageDescription
import Foundation
let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
let libraryDirectory = ProcessInfo.processInfo.environment["DESKFUSE_RUST_LIBRARY_DIR"] ?? root.appendingPathComponent("target/debug").path
let package = Package(
    name: "DeskFuse", platforms: [.macOS(.v14)],
    products: [.executable(name: "deskfuse-app", targets: ["DeskFuse"])],
    dependencies: [
        .package(url: "https://github.com/jaywcjlove/PermissionFlow.git", from: "2.11.2")
    ],
    targets: [
        .target(name: "DeskFuseRemoteApplication", path: "RemoteApplication/Sources/DeskFuseRemoteApplication",
            exclude: ["RemoteTransport.swift", "RemoteTLSIdentity.swift"],
            linkerSettings: [.linkedFramework("ScreenCaptureKit"), .linkedFramework("VideoToolbox"), .linkedFramework("MetalKit"), .linkedFramework("CoreImage")]),
        .systemLibrary(name: "DeskFuseEngineFFI", path: "Sources/DeskFuseEngineFFI"),
        .target(name: "DeskFuseEngine", dependencies: ["DeskFuseEngineFFI"],
                linkerSettings: [.unsafeFlags([libraryDirectory + "/libdeskfuse_ffi.a"])]),
        .target(name: "DeskFuseHost", dependencies: [], path: "Host"),
        .executableTarget(name: "DeskFuse", dependencies: ["DeskFuseEngine", "DeskFuseHost", "DeskFuseRemoteApplication",
                .product(name: "PermissionFlow", package: "PermissionFlow"),
                .product(name: "PermissionFlowScreenRecordingStatus", package: "PermissionFlow")],
            linkerSettings: [.linkedFramework("AppKit"), .linkedFramework("OSAKit"), .linkedFramework("CoreGraphics"), .linkedFramework("CoreFoundation"),
                .linkedFramework("Security"), .linkedFramework("SystemConfiguration"), .linkedFramework("CoreWLAN"), .linkedFramework("CoreLocation"),
                .linkedLibrary("sqlite3"), .linkedLibrary("resolv"), .linkedLibrary("iconv")])
    ]
)
