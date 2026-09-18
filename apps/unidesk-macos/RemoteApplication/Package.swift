// swift-tools-version: 6.0
import PackageDescription

// Kept independent of the shipping client until the P0 two-device gate passes.
let package = Package(
    name: "UniDeskRemoteApplication",
    platforms: [.macOS("14.2")],
    products: [
        .library(name: "UniDeskRemoteApplication", targets: ["UniDeskRemoteApplication"]),
        .executable(name: "unidesk-remote-spike", targets: ["RemoteApplicationSpike"]),
        .executable(name: "unidesk-media-probe", targets: ["RemoteMediaProbe"]),
    ],
    targets: [
        .target(name: "UniDeskRemoteApplication", linkerSettings: [
            .linkedFramework("ScreenCaptureKit"), .linkedFramework("VideoToolbox"),
            .linkedFramework("MetalKit"), .linkedFramework("CoreImage"),
        ]),
        .executableTarget(name: "RemoteApplicationSpike", dependencies: ["UniDeskRemoteApplication"]),
        .executableTarget(name: "RemoteMediaProbe", dependencies: ["UniDeskRemoteApplication"]),
        .testTarget(name: "RemoteApplicationTests", dependencies: ["UniDeskRemoteApplication"]),
    ]
)
