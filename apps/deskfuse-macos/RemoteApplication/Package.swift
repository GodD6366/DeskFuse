// swift-tools-version: 6.0
import PackageDescription

// Kept independent of the shipping client until the P0 two-device gate passes.
let package = Package(
    name: "DeskFuseRemoteApplication",
    platforms: [.macOS("14.2")],
    products: [
        .library(name: "DeskFuseRemoteApplication", targets: ["DeskFuseRemoteApplication"]),
        .executable(name: "deskfuse-remote-spike", targets: ["RemoteApplicationSpike"]),
        .executable(name: "deskfuse-media-probe", targets: ["RemoteMediaProbe"]),
    ],
    targets: [
        .target(name: "DeskFuseRemoteApplication", linkerSettings: [
            .linkedFramework("ScreenCaptureKit"), .linkedFramework("VideoToolbox"),
            .linkedFramework("MetalKit"), .linkedFramework("CoreImage"),
        ]),
        .executableTarget(name: "RemoteApplicationSpike", dependencies: ["DeskFuseRemoteApplication"]),
        .executableTarget(name: "RemoteMediaProbe", dependencies: ["DeskFuseRemoteApplication"]),
        .testTarget(name: "RemoteApplicationTests", dependencies: ["DeskFuseRemoteApplication"]),
    ]
)
