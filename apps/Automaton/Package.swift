// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "Automaton",
    platforms: [.macOS(.v14)],
    targets: [.executableTarget(name: "Automaton", path: "Sources/Automaton")]
)
