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
            linkerSettings: [
                // The Rust static library ships in aimux_ffi.xcframework and is
                // consumed from Dart via DynamicLibrary.process(). Nothing in
                // Swift references its symbols, so link the archive fully —
                // otherwise dead-stripping drops the objects and symbol lookup
                // fails at runtime. -all_load applies to every static archive
                // in the app link (harmless; Flutter's engine is dynamic).
                .unsafeFlags(["-all_load"])
            ]
        ),
        .binaryTarget(
            name: "aimux_ffi",
            path: "../aimux_ffi.xcframework"
        )
    ]
)
