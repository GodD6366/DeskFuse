// swift-tools-version: 6.0
import PackageDescription
import Foundation

// Kept independent of the shipping client until the P0 two-device gate passes.
let freerdpLibDirectory = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("Vendor/FreeRDP/lib")
    .path

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
        ),
        .executableTarget(name: "RemoteApplicationSpike", dependencies: ["DeskFuseRemoteApplication"]),
        .executableTarget(name: "RemoteMediaProbe", dependencies: ["DeskFuseRemoteApplication"]),
        .testTarget(name: "RemoteApplicationTests", dependencies: ["DeskFuseRemoteApplication"]),
    ]
)
