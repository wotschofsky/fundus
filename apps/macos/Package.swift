// swift-tools-version: 6.0
import PackageDescription
let package = Package(name: "VaultUploader", platforms: [.macOS(.v15)], dependencies: [.package(path: "../../native/apple")], targets: [.executableTarget(name: "VaultUploader", dependencies: [.product(name: "VaultNative", package: "apple")]), .testTarget(name: "VaultUploaderTests", dependencies: ["VaultUploader"])])
