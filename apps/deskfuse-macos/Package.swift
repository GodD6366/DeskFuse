// swift-tools-version: 6.0
import PackageDescription
import Foundation

let root = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .deletingLastPathComponent()
    .deletingLastPathComponent()
let libraryDirectory = ProcessInfo.processInfo.environment["DESKFUSE_RUST_LIBRARY_DIR"]
    ?? root.appendingPathComponent("target/debug").path
let freerdpLibDirectory = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("RemoteApplication/Vendor/FreeRDP/lib")
    .path

let remoteApplicationLinkerSettings: [LinkerSetting] = [
    .unsafeFlags(["-L", freerdpLibDirectory]),
    .linkedLibrary("z"),
    .linkedFramework("IOKit"),
    .linkedFramework("Foundation"),
    .linkedFramework("CoreServices"),
    .linkedFramework("Security"),
    .linkedFramework("ScreenCaptureKit"),
    .linkedFramework("VideoToolbox"),
    .linkedFramework("MetalKit"),
    .linkedFramework("CoreImage"),
]

let applicationDependencies: [Target.Dependency] = [
    "MicrophoneNative",
    "DeskFuseEngine",
    "DeskFuseHost",
    "DeskFuseRemoteApplication",
    .product(name: "PermissionFlow", package: "PermissionFlow"),
    .product(name: "PermissionFlowScreenRecordingStatus", package: "PermissionFlow"),
]

let applicationLinkerSettings: [LinkerSetting] = [
    .unsafeFlags(["-L" + root.appendingPathComponent("target/roc-microphone/" + (ProcessInfo.processInfo.environment["DESKFUSE_MACOS_ARCH"] ?? "arm64")).path, "-lroc-microphone", "-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks"]),
    .linkedFramework("AVFoundation"),
    .linkedFramework("AudioToolbox"),
    .linkedFramework("CoreAudio"),
    .linkedFramework("AppKit"),
    .linkedFramework("OSAKit"),
    .linkedFramework("CoreGraphics"),
    .linkedFramework("CoreFoundation"),
    .linkedFramework("Security"),
    .linkedFramework("SystemConfiguration"),
    .linkedFramework("CoreWLAN"),
    .linkedFramework("CoreLocation"),
    .linkedLibrary("sqlite3"),
    .linkedLibrary("resolv"),
    .linkedLibrary("iconv"),
]

let deskfuseTargets: [Target] = [
    .systemLibrary(name: "MicrophoneNative", path: "MicrophoneNative/include"),
    .target(name: "DeskFuseVirtualDisplay", path: "RemoteApplication/Sources/DeskFuseVirtualDisplay",
        cSettings: [.unsafeFlags(["-fobjc-arc"])],
        linkerSettings: [.linkedFramework("CoreGraphics")]),
    .systemLibrary(name: "CDeskFuseRdp", path: "RemoteApplication/Vendor/FreeRDP"),
    .target(
        name: "DeskFuseRemoteApplication",
        dependencies: ["CDeskFuseRdp", "DeskFuseVirtualDisplay"],
        path: "RemoteApplication/Sources/DeskFuseRemoteApplication",
        exclude: ["RemoteTransport.swift", "RemoteTLSIdentity.swift", "RdpBridge"],
        linkerSettings: remoteApplicationLinkerSettings
    ),
    .systemLibrary(name: "DeskFuseEngineFFI", path: "Sources/DeskFuseEngineFFI"),
    .target(
        name: "DeskFuseEngine",
        dependencies: ["DeskFuseEngineFFI"],
        linkerSettings: [.unsafeFlags([libraryDirectory + "/libdeskfuse_ffi.a"])]
    ),
    .target(name: "DeskFuseHost", dependencies: [], path: "Host"),
    .executableTarget(
        name: "DeskFuse",
        dependencies: applicationDependencies,
        linkerSettings: applicationLinkerSettings
    ),
    .executableTarget(
        name: "DeskFuseOCRHelper",
        path: "Sources/DeskFuseOCRHelper",
        linkerSettings: [
            .linkedFramework("ImageIO"),
            .linkedFramework("Vision"),
        ]
    ),
    .testTarget(name: "DeskFuseTests", dependencies: ["DeskFuse", "DeskFuseEngine"], path: "Tests/DeskFuseTests"),
]

let package = Package(
    name: "DeskFuse",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "deskfuse-app", targets: ["DeskFuse"]),
        .executable(name: "deskfuse-ocr-helper", targets: ["DeskFuseOCRHelper"]),
    ],
    dependencies: [
        .package(url: "https://github.com/jaywcjlove/PermissionFlow.git", from: "2.11.2"),
    ],
    targets: deskfuseTargets

)
