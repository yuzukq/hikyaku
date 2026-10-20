// キーの組み合わせ（hikyaku から送る形）と、Mac のキーコード → HID の Usage ID の対応。
//
// 設定画面で押して記録するときは、NSEvent の keyCode（Mac の仮想キーコード kVK_*）が取れる。
// hikyaku に送るのは HID の Usage ID（Keyboard/Keypad Page）なので、ここで変換する。

import Foundation

/// 修飾キー + 1 キー。ビットはブートキーボードと同じ（bit0 Ctrl, 1 Shift, 2 Option, 3 Command。4〜7 は右側）。
public struct KeyCombo: Codable, Equatable, Hashable, Sendable {
    public var modifiers: UInt8
    /// HID の Usage ID（0 なら修飾キーだけ）
    public var usage: UInt8

    public init(modifiers: UInt8, usage: UInt8) {
        self.modifiers = modifiers
        self.usage = usage
    }

    public static let control: UInt8 = 0x01
    public static let shift: UInt8 = 0x02
    public static let option: UInt8 = 0x04
    public static let command: UInt8 = 0x08

    /// 「⌃⌥⇧⌘K」のような表示（Mac のメニューと同じ順）。左右は区別しない。
    public var label: String {
        let m = modifiers | (modifiers >> 4)
        var s = ""
        if m & KeyCombo.control != 0 { s += "⌃" }
        if m & KeyCombo.option != 0 { s += "⌥" }
        if m & KeyCombo.shift != 0 { s += "⇧" }
        if m & KeyCombo.command != 0 { s += "⌘" }
        if usage != 0 { s += HIDKey.name(usage) }
        return s.isEmpty ? "なし" : s
    }
}

public enum HIDKey {
    /// Mac の仮想キーコード（kVK_*）→ HID の Usage ID。対応がなければ nil。
    public static func usage(macKeyCode: UInt16) -> UInt8? {
        macToHID[macKeyCode]
    }

    public static func name(_ usage: UInt8) -> String {
        names[usage] ?? String(format: "0x%02X", usage)
    }

    private static let macToHID: [UInt16: UInt8] = {
        var t: [UInt16: UInt8] = [:]
        // 文字（kVK_ANSI_A …）: Mac のキーコードは配列順ではない
        let letters: [(UInt16, Character)] = [
            (0x00, "A"), (0x0B, "B"), (0x08, "C"), (0x02, "D"), (0x0E, "E"), (0x03, "F"), (0x05, "G"),
            (0x04, "H"), (0x22, "I"), (0x26, "J"), (0x28, "K"), (0x25, "L"), (0x2E, "M"), (0x2D, "N"),
            (0x1F, "O"), (0x23, "P"), (0x0C, "Q"), (0x0F, "R"), (0x01, "S"), (0x11, "T"), (0x20, "U"),
            (0x09, "V"), (0x0D, "W"), (0x07, "X"), (0x10, "Y"), (0x06, "Z"),
        ]
        for (mac, c) in letters {
            t[mac] = 0x04 + UInt8(c.asciiValue! - Character("A").asciiValue!)
        }
        // 数字: HID は 1〜9 が 0x1E〜0x26、0 が 0x27
        let digits: [UInt16] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19] // 0〜9
        for (n, mac) in digits.enumerated() {
            t[mac] = n == 0 ? 0x27 : 0x1D + UInt8(n)
        }
        let others: [UInt16: UInt8] = [
            0x24: 0x28, // Return
            0x35: 0x29, // Escape
            0x33: 0x2A, // Delete（Backspace）
            0x30: 0x2B, // Tab
            0x31: 0x2C, // Space
            0x1B: 0x2D, // -
            0x18: 0x2E, // =
            0x21: 0x2F, // [
            0x1E: 0x30, // ]
            0x2A: 0x31, // \
            0x29: 0x33, // ;
            0x27: 0x34, // '
            0x32: 0x35, // `
            0x2B: 0x36, // ,
            0x2F: 0x37, // .
            0x2C: 0x38, // /
            0x7A: 0x3A, 0x78: 0x3B, 0x63: 0x3C, 0x76: 0x3D, 0x60: 0x3E, 0x61: 0x3F, // F1〜F6
            0x62: 0x40, 0x64: 0x41, 0x65: 0x42, 0x6D: 0x43, 0x67: 0x44, 0x6F: 0x45, // F7〜F12
            0x72: 0x49, // Help（Insert の位置）
            0x73: 0x4A, // Home
            0x74: 0x4B, // Page Up
            0x75: 0x4C, // Forward Delete
            0x77: 0x4D, // End
            0x79: 0x4E, // Page Down
            0x7C: 0x4F, // →
            0x7B: 0x50, // ←
            0x7D: 0x51, // ↓
            0x7E: 0x52, // ↑
            0x47: 0x53, // Keypad Clear
            0x4B: 0x54, 0x43: 0x55, 0x4E: 0x56, 0x45: 0x57, 0x4C: 0x58, // Keypad / * - + Enter
            0x53: 0x59, 0x54: 0x5A, 0x55: 0x5B, 0x56: 0x5C, 0x57: 0x5D, // Keypad 1〜5
            0x58: 0x5E, 0x59: 0x5F, 0x5B: 0x60, 0x5C: 0x61, 0x52: 0x62, // Keypad 6〜9, 0
            0x41: 0x63, // Keypad .
            0x51: 0x67, // Keypad =
            0x69: 0x68, 0x6B: 0x69, 0x71: 0x6A, 0x6A: 0x6B, // F13〜F16
            0x40: 0x6C, 0x4F: 0x6D, 0x50: 0x6E, 0x5A: 0x6F, // F17〜F20
            0x5F: 0x85, // JIS Keypad ,
            0x5E: 0x87, // JIS _（International1）
            0x5D: 0x89, // JIS ¥（International3）
            0x68: 0x90, // かな（LANG1）
            0x66: 0x91, // 英数（LANG2）
        ]
        t.merge(others) { a, _ in a }
        return t
    }()

    private static let names: [UInt8: String] = {
        var n: [UInt8: String] = [:]
        for i in 0..<26 {
            n[0x04 + UInt8(i)] = String(UnicodeScalar(UInt8(ascii: "A") + UInt8(i)))
        }
        for i in 1...9 {
            n[0x1D + UInt8(i)] = String(i)
        }
        n[0x27] = "0"
        for i in 1...12 {
            n[0x39 + UInt8(i)] = "F\(i)"
        }
        for i in 13...24 {
            n[0x68 + UInt8(i - 13)] = "F\(i)"
        }
        let others: [UInt8: String] = [
            0x28: "↩", 0x29: "⎋", 0x2A: "⌫", 0x2B: "⇥", 0x2C: "Space", 0x2D: "-", 0x2E: "=", 0x2F: "[",
            0x30: "]", 0x31: "\\", 0x33: ";", 0x34: "'", 0x35: "`", 0x36: ",", 0x37: ".", 0x38: "/",
            0x49: "Help", 0x4A: "↖", 0x4B: "⇞", 0x4C: "⌦", 0x4D: "↘", 0x4E: "⇟",
            0x4F: "→", 0x50: "←", 0x51: "↓", 0x52: "↑", 0x53: "⌧",
            0x54: "Keypad /", 0x55: "Keypad *", 0x56: "Keypad -", 0x57: "Keypad +", 0x58: "⌤",
            0x59: "Keypad 1", 0x5A: "Keypad 2", 0x5B: "Keypad 3", 0x5C: "Keypad 4", 0x5D: "Keypad 5",
            0x5E: "Keypad 6", 0x5F: "Keypad 7", 0x60: "Keypad 8", 0x61: "Keypad 9", 0x62: "Keypad 0",
            0x63: "Keypad .", 0x67: "Keypad =", 0x85: "Keypad ,", 0x87: "_", 0x89: "¥", 0x90: "かな", 0x91: "英数",
        ]
        n.merge(others) { a, _ in a }
        return n
    }()
}
