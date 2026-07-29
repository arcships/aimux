// swift-tools-version: 5.9
// Package.swift — aimux Swift binding (C ABI path via aimux-ffi)

import PackageDescription

let package = Package(
    name: "Aimux",
    platforms: [
        .iOS(.v15),
        .macOS(.v12),
    ],
    products: [
        .library(name: "Aimux", targets: ["Aimux"]),
    ],
    targets: [
        // The C header from aimux-ffi is imported as a system module.
        // In production, the prebuilt .xcframework provides the binary.
        // For development, point `path` to the built library.
        .systemLibrary(
            name: "CAimuxFFI",
            path: "Sources/CAimuxFFI",
            pkgConfig: "aimux-ffi"
        ),
        .target(
            name: "Aimux",
            dependencies: ["CAimuxFFI"],
            path: "Sources/Aimux"
        ),
        .testTarget(
            name: "AimuxTests",
            dependencies: ["Aimux"],
            path: "Tests/AimuxTests"
        ),
    ]
)
