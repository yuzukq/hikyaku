//! トラックパッドの入力を、マウスレポートに変換する。
//!
//! ハードウェアに依存しない純粋なロジック。`no_std`・時刻は呼び出し側が与える。
//! Pinnacle の相対モードのパケットを扱う [`relative`]、ショートカットリングの [`ring`]、本体のボタンの判定 [`button`]。
//! 絶対座標から移動量を作る旧方式（`Pointer`・`tracker`）は 2026-10-13 に削除した（タグ `absolute-mode-last` に残っている）。

#![cfg_attr(not(test), no_std)]

pub mod button;
pub mod relative;
pub mod ring;

/// hikyaku 本体の置き方。基準の向き（2026-10 までの向き。基板裏の印字が読める向き）から、**時計回りに** 90° 単位で回した量。
/// キーボードによって本体を回して置くときに、カーソル・スクロール・リングの向きを合わせる。
/// 例: 左に 90° 倒して置く = [`Rotation::R270`]。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rotation {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

impl Rotation {
    /// 時計回りに 90° を何回か（0〜3）から。それ以外は None。
    pub const fn from_quarters(q: u8) -> Option<Rotation> {
        match q {
            0 => Some(Rotation::R0),
            1 => Some(Rotation::R90),
            2 => Some(Rotation::R180),
            3 => Some(Rotation::R270),
            _ => None,
        }
    }

    pub const fn quarters(self) -> u8 {
        self as u8
    }

    /// 本体から見た画面の向きの移動量（x 右・y 下）→ 置いた向きでの画面の移動量。
    /// 時計回りに 90° 回して置くと、本体の「上」は画面の右を向く。
    pub const fn apply(self, x: i32, y: i32) -> (i32, i32) {
        match self {
            Rotation::R0 => (x, y),
            Rotation::R90 => (-y, x),
            Rotation::R180 => (-x, -y),
            Rotation::R270 => (y, -x),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Rotation;

    #[test]
    fn rotation_turns_the_up_direction_clockwise() {
        // 本体の上（0, -1）が、置いた向きでどちらを向くか
        assert_eq!(Rotation::R0.apply(0, -1), (0, -1)); // 上
        assert_eq!(Rotation::R90.apply(0, -1), (1, 0)); // 右
        assert_eq!(Rotation::R180.apply(0, -1), (0, 1)); // 下
        assert_eq!(Rotation::R270.apply(0, -1), (-1, 0)); // 左（左に 90° 倒した）
        for q in 0..4 {
            assert_eq!(Rotation::from_quarters(q).unwrap().quarters(), q);
        }
        assert_eq!(Rotation::from_quarters(4), None);
    }
}
