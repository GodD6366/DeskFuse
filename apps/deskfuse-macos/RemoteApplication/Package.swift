// swift-tools-version: 6.0
import PackageDescription
import Foundation

// Kept independent of the shipping client until the P0 two-device gate passes.
let freerdpLibDirectory = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("Vendor/FreeRDP/lib")
    .path
let homebrewPrefix = ProcessInfo.processInfo.environment["HOMEBREW_PREFIX"] ?? "/opt/homebrew"
let systemLibDirectories = [
    homebrewPrefix + "/lib",
    homebrewPrefix + "/opt/openssl@3/lib",
    homebrewPrefix + "/opt/json-c/lib",
    homebrewPrefix + "/opt/libusb/lib",
]

let package = Package(
    name: "DeskFuseRemoteApplication",
    platforms: [.macOS("14.2")],
    products: [
        .library(name: "DeskFuseRemoteApplication", targets: ["DeskFuseRemoteApplication"]),
        .executable(name: "deskfuse-remote-spike", targets: ["RemoteApplicationSpike"]),
        .executable(name: "deskfuse-media-probe", targets: ["RemoteMediaProbe"]),
    ],
    targets: [
        .systemLibrary(name: "CDeskFuseRdp", path: "Vendor/FreeRDP"),
        .target(
            name: "DeskFuseRemoteApplication",
            dependencies: ["CDeskFuseRdp"],
            exclude: ["RdpBridge"],
            linkerSettings: [
                .unsafeFlags(["-L", freerdpLibDirectory] + systemLibDirectories.flatMap { ["-L", $0] }),
                .linkedLibrary("ssl"),
                .linkedLibrary("crypto"),
                .linkedLibrary("usb-1.0"),
                .linkedLibrary("json-c"),
                .linkedFramework("Foundation"),
                .linkedFramework("CoreServices"),
                .linkedFramework("Security"),
                .linkedFramework("ScreenCaptureKit"),
                .linkedFramework("VideoToolbox"),
                .linkedFramework("MetalKit"),
                .linkedFramework("CoreImage"),
            ]
        ),
        .executableTarget(name: "RemoteApplicationSpike", dependencies: ["DeskFuseRemoteApplication"]),
        .executableTarget(name: "RemoteMediaProbe", dependencies: ["DeskFuseRemoteApplication"]),
        .testTarget(name: "RemoteApplicationTests", dependencies: ["DeskFuseRemoteApplication"]),
    ]
)
