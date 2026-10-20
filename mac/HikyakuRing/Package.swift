// swift-tools-version: 6.0
// hikyaku のショートカットリング（Mac アプリ）。仕様: docs/design/ring.md
// ビルドと起動は build.sh（Bluetooth の許可に Info.plist が要るので、.app にまとめて起動する）。

import PackageDescription

let package = Package(
    name: "HikyakuRing",
    platforms: [.macOS(.v14)],
    targets: [
        // 通信の形式・リングの状態遷移・設定。AppKit に依存せず、テストできる部分。
        .target(name: "RingCore"),
        .executableTarget(
            name: "HikyakuRing",
            dependencies: ["RingCore"],
            // CoreBluetooth・AppKit のデリゲートと Swift 6 の厳密な並行性検査が噛み合わないので、アプリ側は Swift 5 モード
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .testTarget(name: "RingCoreTests", dependencies: ["RingCore"]),
    ]
)
