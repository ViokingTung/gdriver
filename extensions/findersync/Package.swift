// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "GDriverFinderSync",
    platforms: [
        .macOS(.v12),
    ],
    products: [
        .library(
            name: "GDriverFinderSync",
            type: .dynamic,
            targets: ["GDriverFinderSync"]
        ),
    ],
    dependencies: [],
    targets: [
        .target(
            name: "GDriverFinderSync",
            path: "Sources",
            swiftSettings: [
                .unsafeFlags(["-application-extension"]),
            ]
        ),
    ]
)
