// swift-tools-version: 6.0
import PackageDescription
let package = Package(name: "VaultNative", platforms: [.macOS(.v15), .iOS(.v17)], products: [.library(name: "VaultNative", targets: ["VaultNative"])], targets: [.target(name: "VaultNative"), .testTarget(name: "VaultNativeTests", dependencies: ["VaultNative"])])
