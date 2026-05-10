// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "GDriverFileProvider",
    platforms: [
        .macOS(.v12),
    ],
    products: [
        .executable(
            name: "GDriverFileProvider",
            targets: ["GDriverFileProvider"]
        ),
    ],
    dependencies: [],
    targets: [
        .executableTarget(
            name: "GDriverFileProvider",
            path: "Sources",
            swiftSettings: [
                .unsafeFlags(["-application-extension"]),
            ]
        ),
    ]
)
