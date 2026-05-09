// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "GDriverFileProvider",
    platforms: [
        .macOS(.v12),
    ],
    products: [
        .library(
            name: "GDriverFileProvider",
            type: .dynamic,
            targets: ["GDriverFileProvider"]
        ),
    ],
    dependencies: [],
    targets: [
        .target(
            name: "GDriverFileProvider",
            path: "Sources",
            swiftSettings: [
                .unsafeFlags(["-application-extension"]),
            ]
        ),
    ]
)
