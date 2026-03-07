// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "turm-macos",
    platforms: [
        .macOS(.v14),
    ],
    targets: [
        .executableTarget(
            name: "Turm",
            path: "Sources/Turm"
        ),
    ]
)
