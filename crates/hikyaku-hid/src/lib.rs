//! USB キーボードの HID レポートを解釈し、固定の出力レポートへ詰め替える。
//! マイコン側でもそのまま使えるよう `no_std`。

#![cfg_attr(not(test), no_std)]

/// ブートプロトコルのキーボード入力レポート（8 バイト）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BootKeyboardReport {
    pub modifiers: u8,
    pub keys: [u8; 6],
}

impl BootKeyboardReport {
    pub fn parse(raw: &[u8]) -> Option<Self> {
        if raw.len() < 8 {
            return None;
        }
        let mut keys = [0u8; 6];
        keys.copy_from_slice(&raw[2..8]);
        Some(Self { modifiers: raw[0], keys })
    }

    /// 押されているキー（0 と ErrorRollOver=1 は除く）。
    pub fn pressed(&self) -> impl Iterator<Item = u8> + '_ {
        self.keys.iter().copied().filter(|&k| k > 1)
    }

    pub fn is_rollover_error(&self) -> bool {
        self.keys[0] == 1
    }
}

pub const MOD_NAMES: [&str; 8] = ["LCtrl", "LShift", "LAlt", "LGui", "RCtrl", "RShift", "RAlt", "RGui"];

/// Usage ID（Keyboard/Keypad ページ）の名前。よく使うものだけ。
pub fn usage_name(code: u8) -> &'static str {
    match code {
        0x04..=0x1d => LETTERS[(code - 0x04) as usize],
        0x1e..=0x27 => DIGITS[(code - 0x1e) as usize],
        0x28 => "Enter",
        0x29 => "Esc",
        0x2a => "Backspace",
        0x2b => "Tab",
        0x2c => "Space",
        0x39 => "CapsLock",
        0x3a..=0x45 => FKEYS[(code - 0x3a) as usize],
        0x4f => "Right",
        0x50 => "Left",
        0x51 => "Down",
        0x52 => "Up",
        0x87 => "Intl1(Ro)",
        0x88 => "Intl2(Kana)",
        0x89 => "Intl3(Yen)",
        0x8a => "Intl4(Henkan)",
        0x8b => "Intl5(Muhenkan)",
        0x90 => "Lang1(Kana/かな)",
        0x91 => "Lang2(Eisu/英数)",
        _ => "?",
    }
}

/// F24 のキーコード（HID Usage 0x73）。hikyaku はこのキーを Mac に送らず、押している間トラックパッドをスクロールにする。
pub const KEY_F24: u8 = 0x73;
/// F23（0x72）。hikyaku はこのキーを Mac に送らず、押すたびにショートカットリングを開閉する。
pub const KEY_F23: u8 = 0x72;
/// F21 / F22（0x70 / 0x71）: 送り先（プロファイル）の 1 台目 / 2 台目に切り替える（docs/design/multi-host.md）。ホストには送らない。
pub const KEY_F21: u8 = 0x70;
pub const KEY_F22: u8 = 0x71;
/// Esc（0x29）。リング中だけ、リングを閉じるのに使う。
pub const KEY_ESC: u8 = 0x29;

/// ブートキーボードのレポート（8 バイト）からキー `key` を取り除き、残りのキーを前に詰める。取り除いたら true。
/// 修飾キー（1 バイト目）と予約バイト（2 バイト目）は触らない。
pub fn take_key(report: &mut [u8; 8], key: u8) -> bool {
    let keys = &mut report[2..];
    let Some(i) = keys.iter().position(|&k| k == key) else {
        return false;
    };
    keys.copy_within(i + 1.., i);
    keys[keys.len() - 1] = 0;
    true
}

const LETTERS: [&str; 26] = [
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M",
    "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
];
const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
const FKEYS: [&str; 12] = ["F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_boot_report() {
        let r = BootKeyboardReport::parse(&[0x02, 0, 0x04, 0x05, 0, 0, 0, 0]).unwrap();
        assert_eq!(r.modifiers, 0x02);
        assert_eq!(r.pressed().collect::<Vec<_>>(), vec![4, 5]);
        assert_eq!(usage_name(4), "A");
    }

    #[test]
    fn take_key_removes_and_compacts() {
        let mut r = [0x02, 0, 0x04, KEY_F24, 0x05, 0, 0, 0];
        assert!(take_key(&mut r, KEY_F24));
        assert_eq!(r, [0x02, 0, 0x04, 0x05, 0, 0, 0, 0]);
    }

    #[test]
    fn take_key_last_slot_and_absent() {
        let mut r = [0, 0, 1, 2, 3, 4, 5, KEY_F24];
        assert!(take_key(&mut r, KEY_F24));
        assert_eq!(r, [0, 0, 1, 2, 3, 4, 5, 0]);
        let mut r = [0x01, 0, 0x04, 0, 0, 0, 0, 0];
        assert!(!take_key(&mut r, KEY_F24));
        assert_eq!(r, [0x01, 0, 0x04, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn rejects_short() {
        assert!(BootKeyboardReport::parse(&[0; 4]).is_none());
    }
}
