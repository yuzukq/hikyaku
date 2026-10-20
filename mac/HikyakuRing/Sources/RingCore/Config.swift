// リングの項目の割り当てと調整値。~/Library/Application Support/HikyakuRing/config.json に保存する。

import Foundation

/// 連続値の項目（指を離すとノブ操作モードになる）。
public enum KnobKind: String, Codable, CaseIterable, Sendable {
    case volume
    case brightness
    case seek

    public var title: String {
        switch self {
        case .volume: "音量"
        case .brightness: "画面の明るさ"
        case .seek: "シーク（Spotify・ミュージック）"
        }
    }
}

/// メディアキーで実行する、1 回きりの項目。
public enum MediaKey: String, Codable, CaseIterable, Sendable {
    case playPause
    case nextTrack
    case previousTrack
    case mute

    public var usage: UInt16 {
        switch self {
        case .playPause: ConsumerUsage.playPause
        case .nextTrack: ConsumerUsage.nextTrack
        case .previousTrack: ConsumerUsage.previousTrack
        case .mute: ConsumerUsage.mute
        }
    }

    public var title: String {
        switch self {
        case .playPause: "再生 / 一時停止"
        case .nextTrack: "次の曲"
        case .previousTrack: "前の曲"
        case .mute: "消音"
        }
    }
}

public enum RingAction: Codable, Equatable, Hashable, Sendable {
    case none
    case keys(KeyCombo)
    case media(MediaKey)
    case knob(KnobKind)

    public var isKnob: Bool {
        if case .knob = self { true } else { false }
    }
}

public struct RingItem: Codable, Equatable, Hashable, Sendable {
    public var name: String
    /// SF Symbols の名前
    public var symbol: String
    public var action: RingAction

    public init(name: String, symbol: String, action: RingAction) {
        self.name = name
        self.symbol = symbol
        self.action = action
    }

    public static let empty = RingItem(name: "", symbol: "circle.dashed", action: .none)
}

/// アプリ側で持つ調整値（ファームの書き換えなしで試せる）。
public struct RingTuning: Codable, Equatable, Sendable {
    /// これより中心に近い位置はキャンセル範囲（パッドの縁 ≒ 1000）
    public var cancelRadius: Int = 350
    /// 項目の境目のヒステリシス（度）。今の項目から、境目をこれだけ越えるまで切り替えない
    public var hysteresisDegrees: Double = 6
    /// 指を離したとき、この時間だけ前の位置で項目を判定する（ミリ秒）
    public var releaseLookbackMs: Double = 50
    /// ノブ操作で 1 段階に要する回転（度）
    public var knobStepDegrees: Double = 15
    /// ノブモードを抜けるタップの長さの上限（ミリ秒）
    public var tapMaxMs: Double = 250
    /// タップとみなす動きの上限（パッドの縁 ≒ 1000 の単位）
    public var tapMaxMove: Int = 150
    /// シークの 1 段階（秒）
    public var seekStepSeconds: Double = 5

    public init() {}
}

public struct RingConfig: Codable, Equatable, Sendable {
    /// 8 か所。0 = 上、時計回りに 45° ずつ
    public var items: [RingItem]
    public var tuning: RingTuning

    public init(items: [RingItem], tuning: RingTuning = RingTuning()) {
        self.items = RingConfig.padded(items)
        self.tuning = tuning
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        items = RingConfig.padded(try c.decode([RingItem].self, forKey: .items))
        // 後から足した調整値が古い設定ファイルになくても読めるように
        tuning = (try? c.decode(RingTuning.self, forKey: .tuning)) ?? RingTuning()
    }

    public static let slotCount = 8

    private static func padded(_ items: [RingItem]) -> [RingItem] {
        Array((items + Array(repeating: .empty, count: slotCount)).prefix(slotCount))
    }

    public static let standard = RingConfig(items: [
        RingItem(name: "音量", symbol: "speaker.wave.2", action: .knob(.volume)),
        RingItem(name: "次の曲", symbol: "forward.end", action: .media(.nextTrack)),
        RingItem(name: "スクリーンショット", symbol: "camera.viewfinder",
                 action: .keys(KeyCombo(modifiers: KeyCombo.command | KeyCombo.shift, usage: 0x22))), // ⇧⌘5
        RingItem(name: "シーク", symbol: "gobackward", action: .knob(.seek)),
        RingItem(name: "再生 / 停止", symbol: "playpause", action: .media(.playPause)),
        RingItem(name: "Mission Control", symbol: "rectangle.3.group",
                 action: .keys(KeyCombo(modifiers: KeyCombo.control, usage: 0x52))), // ⌃↑
        RingItem(name: "明るさ", symbol: "sun.max", action: .knob(.brightness)),
        RingItem(name: "Spotlight", symbol: "magnifyingglass",
                 action: .keys(KeyCombo(modifiers: KeyCombo.command, usage: 0x2C))), // ⌘Space
    ])
}

public enum ConfigStore {
    public static var defaultURL: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("HikyakuRing", isDirectory: true)
            .appendingPathComponent("config.json")
    }

    /// 読めなければ（初回・壊れている）標準の割り当て。
    public static func load(from url: URL = defaultURL) -> RingConfig {
        guard let data = try? Data(contentsOf: url),
              let config = try? JSONDecoder().decode(RingConfig.self, from: data)
        else { return .standard }
        return config
    }

    public static func save(_ config: RingConfig, to url: URL = defaultURL) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try encoder.encode(config).write(to: url, options: .atomic)
    }
}
