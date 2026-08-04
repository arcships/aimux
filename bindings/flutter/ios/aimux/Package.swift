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
                "aimux_ffi"
            ],
            resources: [
                .process("PrivacyInfo.xcprivacy")
            ],
            // shim.c references every aimux-ffi symbol (see file), so the
            // static archive objects are pulled into the app link even though
            // Dart resolves them at runtime via DynamicLibrary.process().
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
