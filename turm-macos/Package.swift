// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "turm-macos",
    platforms: [
        .macOS(.v14),
    ],
    dependencies: [
        .package(url: "https://github.com/migueldeicaza/SwiftTerm", from: "1.2.0"),
        .package(url: "https://github.com/LebJe/TOMLKit", from: "0.6.0"),
    ],
    targets: [
        .executableTarget(
            name: "Turm",
            dependencies: [
                .product(name: "SwiftTerm", package: "SwiftTerm"),
                .product(name: "TOMLKit", package: "TOMLKit"),
            ],
            path: "Sources/Turm",
        ),
    ],
)
