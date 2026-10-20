// hikyaku の独自サービス（ring）の形式。ファーム側: crates/hikyaku-pointer/src/ring.rs

import Foundation

public enum RingUUID {
    public static let service = "6b9a0010-6f1e-4b8e-9d57-7f0d0a5b0001"
    /// hikyaku → アプリ（read / notify、12 バイト固定）
    public static let state = "6b9a0011-6f1e-4b8e-9d57-7f0d0a5b0001"
    /// アプリ → hikyaku（write / write without response）
    public static let ctrl = "6b9a0012-6f1e-4b8e-9d57-7f0d0a5b0001"
}

/// パッドに触れている指の状態（state の種類 3）。
public struct RingSample: Equatable, Sendable {
    public var touching: Bool
    /// 0° = 上、時計回り（画面上の向き）
    public var angle: Double
    /// 中心からの距離（パッドの縁 ≒ 1000）
    public var radius: Int
    public var x: Int
    public var y: Int
    /// 接触の強さ（離す直前は小さくなる）
    public var z: Int
    public var seq: UInt8

    public init(touching: Bool, angle: Double, radius: Int, x: Int = 0, y: Int = 0, z: Int = 0, seq: UInt8 = 0) {
        self.touching = touching
        self.angle = angle
        self.radius = radius
        self.x = x
        self.y = y
        self.z = z
        self.seq = seq
    }
}

public enum RingEvent: Equatable, Sendable {
    case opened
    case closed
    case position(RingSample)

    /// state の 12 バイトを読む。形式が違えば nil。
    public init?(_ data: Data) {
        let b = [UInt8](data)
        guard b.count >= 12 else { return nil }
        let u16 = { (i: Int) in Int(b[i]) | Int(b[i + 1]) << 8 }
        switch b[0] {
        case 1: self = .opened
        case 2: self = .closed
        case 3:
            self = .position(RingSample(
                touching: b[1] & 1 != 0,
                angle: Double(u16(2)) / 100,
                radius: u16(4),
                x: u16(6),
                y: u16(8),
                z: Int(b[10]),
                seq: b[11]
            ))
        default: return nil
        }
    }
}

/// アプリ → hikyaku の命令（ctrl に書くバイト列）。
public enum RingCommand: Equatable, Sendable {
    /// 接続したら最初に送る。これを受け取るまで F23 でリングは開かない
    case hello
    case close
    case open
    /// キーの組み合わせを押して離す（hikyaku から本物のキーボード入力として届く）
    case key(KeyCombo)
    /// メディアキー（Consumer の Usage）を押して離す
    case consumer(UInt16)

    public var bytes: [UInt8] {
        switch self {
        case .hello: [0x00]
        case .close: [0x01]
        case .open: [0x02]
        case .key(let combo): [0x10, combo.modifiers, combo.usage]
        case .consumer(let usage): [0x11, UInt8(usage & 0xFF), UInt8(usage >> 8)]
        }
    }
}

/// HID Consumer Page の Usage ID。
public enum ConsumerUsage {
    public static let volumeUp: UInt16 = 0x00E9
    public static let volumeDown: UInt16 = 0x00EA
    public static let mute: UInt16 = 0x00E2
    public static let brightnessUp: UInt16 = 0x006F
    public static let brightnessDown: UInt16 = 0x0070
    public static let playPause: UInt16 = 0x00CD
    public static let nextTrack: UInt16 = 0x00B5
    public static let previousTrack: UInt16 = 0x00B6
}
