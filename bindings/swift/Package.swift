// swift-tools-version: 5.9
import PackageDescription
import Foundation

// Test-only C implementation: never included in the shipped Aimux product.
let testABI = ProcessInfo.processInfo.environment["AIMUX_SWIFT_TEST_ABI"] == "1"
var targets: [Target] = [
    .systemLibrary(name: "CAimuxFFI", path: "Sources/CAimuxFFI"),
    .target(name: "Aimux", dependencies: ["CAimuxFFI"], path: ".",
            exclude: ["Tests", "Sources/CAimuxFFI", "scripts", "README.md"],
            sources: ["Sources/Aimux", "generated"],
            linkerSettings: testABI ? [] : [.linkedLibrary("aimux_ffi")]),
    .testTarget(name: "AimuxTests", dependencies: testABI ? ["Aimux", "CAimuxTestABI"] : ["Aimux"]),
]
if testABI {
    targets.append(.target(name: "CAimuxTestABI", path: "Tests/CAimuxTestABI"))
}
let package = Package(
    name: "Aimux",
    platforms: [.iOS(.v15), .macOS(.v12)],
    products: [.library(name: "Aimux", targets: ["Aimux"])],
    targets: targets
)
