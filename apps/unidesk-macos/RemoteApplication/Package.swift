// swift-tools-version: 6.0
import PackageDescription
import Foundation

// Kept independent of the shipping client until the P0 two-device gate passes.
let freerdpLibDirectory = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent()
    .appendingPathComponent("Vendor/FreeRDP/lib")
    .path
let homebrewPrefix = ProcessInfo.processInfo.environment["HOMEBREW_PREFIX"] ?? "/opt/homebrew"
let systemLibDirectories = [homebrewPrefix + "/lib", homebrewPrefix + "/opt/openssl@3/lib", homebrewPrefix + "/opt/json-c/lib", homebrewPrefix + "/opt/libusb/lib"]

let package = Package(
    name: "UniDeskRemoteApplication",
    platforms: [.macOS("14.2")],
    products: [
        .library(name: "UniDeskRemoteApplication", targets: ["UniDeskRemoteApplication"]),
        .executable(name: "unidesk-remote-spike", targets: ["RemoteApplicationSpike"]),
        .executable(name: "unidesk-media-probe", targets: ["RemoteMediaProbe"]),
    ],
    targets: [
        .systemLibrary(name: "CUniDeskRdp", path: "Vendor/FreeRDP"),
        .target(name: "UniDeskRemoteApplication",
            dependencies: ["CUniDeskRdp"],
            exclude: ["RdpBridge"],
            linkerSettings: [
                .unsafeFlags(["-L", freerdpLibDirectory] + systemLibDirectories.flatMap { ["-L", $0] }),
                .linkedLibrary("ssl"), .linkedLibrary("crypto"),
                .linkedLibrary("usb-1.0"), .linkedLibrary("json-c"),
                .linkedFramework("Foundation"), .linkedFramework("CoreServices"), .linkedFramework("Security"),
                .linkedFramework("ScreenCaptureKit"), .linkedFramework("VideoToolbox"),
                .linkedFramework("MetalKit"), .linkedFramework("CoreImage"),
            ]),
        .executableTarget(name: "RemoteApplicationSpike", dependencies: ["UniDeskRemoteApplication"]),
        .executableTarget(name: "RemoteMediaProbe", dependencies: ["UniDeskRemoteApplication"]),
        .testTarget(name: "RemoteApplicationTests", dependencies: ["UniDeskRemoteApplication"]),
    ]
)
