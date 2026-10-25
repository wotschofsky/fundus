// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "TranscriptVault",
    platforms: [.macOS(.v15)],
    dependencies: [.package(path: "../../native/apple")],
    targets: [
        .executableTarget(name: "TranscriptVault", dependencies: [.product(name: "VaultNative", package: "apple")]),
        .testTarget(name: "TranscriptVaultTests", dependencies: ["TranscriptVault"]),
    ]
)
