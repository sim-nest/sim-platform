// swift-tools-version: 5.10
import PackageDescription

let package = Package(
    name: "SimIOSShell",
    platforms: [.iOS(.v16)],
    products: [.library(name: "SimIOSShell", targets: ["SimIOSShell"])],
    targets: [
        .target(name: "CSimPlatformIOS", publicHeadersPath: "include"),
        .target(name: "SimIOSShell", dependencies: ["CSimPlatformIOS"]),
        .testTarget(name: "SimIOSShellTests", dependencies: ["SimIOSShell"]),
    ]
)
