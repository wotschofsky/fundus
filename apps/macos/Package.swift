// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Fundus",
    platforms: [.macOS(.v15)],
    dependencies: [.package(path: "../../native/apple")],
    targets: [
        .executableTarget(name: "Fundus", dependencies: [.product(name: "VaultNative", package: "apple")]),
        .testTarget(name: "FundusTests", dependencies: ["Fundus"]),
    ]
)
