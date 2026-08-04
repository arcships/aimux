// swift-tools-version: 5.9
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "aimux",
    platforms: [
        .iOS("12.0")
    ],
    products: [
        .library(name: "aimux", targets: ["aimux"])
    ],
    dependencies: [
        .package(name: "FlutterFramework", path: "../FlutterFramework")
    ],
    targets: [
        .target(
            name: "aimux",
            dependencies: [
                .product(name: "FlutterFramework", package: "FlutterFramework"),
                "aimux_ffi",
                "aimux_ffi_shim"
            ],
            resources: [
                .process("PrivacyInfo.xcprivacy")
            ]
        ),
        // References every aimux-ffi symbol (see shim.c), pulling the static
        // archive objects into the app link — Dart resolves them at runtime
        // via DynamicLibrary.process() and nothing else references them.
        // Separate C target: SwiftPM rejects mixed-language targets.
        .target(
            name: "aimux_ffi_shim",
            dependencies: ["aimux_ffi"],
            cSettings: [
                .headerSearchPath("include")
            ]
        ),
        .binaryTarget(
            name: "aimux_ffi",
            // Inside the package directory so Flutter's SPM mirroring
            // (ephemeral/Packages) carries the artifact along.
            path: "Sources/aimux_ffi.xcframework"
        )
    ]
)
