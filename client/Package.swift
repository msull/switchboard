// swift-tools-version: 5.10
// The Swift side of docs/remote-client.md §7: four libraries shared by
// the visionOS and iPadOS apps, a command-line browser for development
// on the Mac, and tests that run with `swift test` here.
import PackageDescription

let package = Package(
    name: "SwitchboardClient",
    platforms: [.macOS(.v14), .iOS(.v17), .visionOS(.v1)],
    products: [
        .library(name: "SwitchboardProtocol", targets: ["SwitchboardProtocol"]),
        .library(name: "SwitchboardTransport", targets: ["SwitchboardTransport"]),
        .library(name: "SwitchboardClientCore", targets: ["SwitchboardClientCore"]),
        .library(name: "SwitchboardUI", targets: ["SwitchboardUI"]),
        .executable(name: "switchboard-remote", targets: ["switchboard-remote"]),
    ],
    targets: [
        .target(name: "SwitchboardProtocol"),
        .target(name: "SwitchboardTransport", dependencies: ["SwitchboardProtocol"]),
        .target(
            name: "SwitchboardClientCore",
            dependencies: ["SwitchboardProtocol", "SwitchboardTransport"]),
        .target(name: "SwitchboardUI", dependencies: ["SwitchboardClientCore"]),
        .executableTarget(name: "switchboard-remote", dependencies: ["SwitchboardClientCore"]),
        .testTarget(name: "SwitchboardProtocolTests", dependencies: ["SwitchboardProtocol"]),
        .testTarget(name: "SwitchboardTransportTests", dependencies: ["SwitchboardTransport"]),
        .testTarget(name: "SwitchboardClientCoreTests", dependencies: ["SwitchboardClientCore"]),
    ]
)
