//! Pinnacle の座標パケットから、**下位バイトの前後差だけ**で相対位置を作る。
//!
//! 実機で、パケットの上位ビット（4 バイト目の X[11:8]・Y[11:8]）が下位バイトより遅れて更新され、
//! 桁上がりのたびに値が ±256 跳ぶことが分かった（報告された座標のスパイク 16%）。
//! 一方、下位バイト（2・3 バイト目）の前後差は 250kHz で 0.2% しか乱れない。
//! マウスに必要なのは相対移動なので、上位ビットは使わず、下位バイトの差（256 を法とする符号付き）を足し込む。
//!
//! 1 サンプルの移動が 128 以上だと曖昧になる（256 の自由度）。差が大きいときは、直前の速度に近いほうの解釈を選ぶ。

/// 256 を法とする符号付きの差（-128..=127）。
fn signed_delta(from: u8, to: u8) -> i32 {
    (to.wrapping_sub(from) as i8) as i32
}

/// 追いかけた位置は `ORIGIN` を中心にした u16。接触のたびに戻すので、ずれが蓄積しない。
pub const ORIGIN: i32 = 32768;

/// 差の絶対値がこれ以上のときは、曖昧さの解消（直前の速度に近い解釈）を試す。
const AMBIGUOUS: i32 = 96;

#[derive(Clone, Copy, Debug, Default)]
pub struct LowByteTracker {
    pos: (i32, i32),
    last: Option<(u8, u8)>,
    vel: (i32, i32),
}

impl LowByteTracker {
    pub const fn new() -> Self {
        Self { pos: (ORIGIN, ORIGIN), last: None, vel: (0, 0) }
    }

    /// 指が離れたら呼ぶ。
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    fn step(delta: i32, vel: i32) -> i32 {
        if delta.abs() < AMBIGUOUS {
            return delta;
        }
        // delta と、256 ずらしたもののうち、直前の速度に近いほうを選ぶ。
        let alt = if delta > 0 { delta - 256 } else { delta + 256 };
        if (alt - vel).abs() < (delta - vel).abs() {
            alt
        } else {
            delta
        }
    }

    /// 新しいサンプルの X・Y 下位バイトを与えて、追いかけた位置 (x, y)（u16 の範囲）を返す。
    pub fn update(&mut self, x_lo: u8, y_lo: u8) -> (u16, u16) {
        if let Some((px, py)) = self.last {
            let dx = Self::step(signed_delta(px, x_lo), self.vel.0);
            let dy = Self::step(signed_delta(py, y_lo), self.vel.1);
            self.pos.0 = (self.pos.0 + dx).clamp(0, u16::MAX as i32);
            self.pos.1 = (self.pos.1 + dy).clamp(0, u16::MAX as i32);
            self.vel = (dx, dy);
        }
        self.last = Some((x_lo, y_lo));
        (self.pos.0 as u16, self.pos.1 as u16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_delta_wraps() {
        assert_eq!(signed_delta(0xF0, 0x10), 32);
        assert_eq!(signed_delta(0x10, 0xF0), -32);
        assert_eq!(signed_delta(0x00, 0x7F), 127);
        assert_eq!(signed_delta(0x00, 0x80), -128);
    }

    #[test]
    fn starts_at_origin_and_follows_small_steps() {
        let mut t = LowByteTracker::new();
        assert_eq!(t.update(100, 50), (ORIGIN as u16, ORIGIN as u16));
        assert_eq!(t.update(110, 47), (ORIGIN as u16 + 10, ORIGIN as u16 - 3));
    }

    #[test]
    fn counts_through_low_byte_wrap_without_using_high_bits() {
        // 実機のログの並び: X 下位が 0xfe → 0x0d（桁上がり）。上位ビットを見なくても +15 になる。
        let mut t = LowByteTracker::new();
        t.update(0xee, 0);
        let (x1, _) = t.update(0xfe, 0);
        let (x2, _) = t.update(0x0d, 0);
        assert_eq!(x1 as i32 - ORIGIN, 16);
        assert_eq!(x2 as i32 - ORIGIN, 31);
    }

    #[test]
    fn straight_motion_through_many_wraps_stays_linear() {
        let mut t = LowByteTracker::new();
        let mut lo = 0u8;
        t.update(lo, lo);
        for i in 1..=100 {
            lo = lo.wrapping_add(40);
            let (x, y) = t.update(lo, lo);
            assert_eq!(x as i32 - ORIGIN, 40 * i);
            assert_eq!(y as i32 - ORIGIN, 40 * i);
        }
    }

    #[test]
    fn large_ambiguous_step_follows_previous_velocity() {
        let mut t = LowByteTracker::new();
        let mut lo = 0u8;
        t.update(lo, 0);
        // 速度 110 で走っている（曖昧さの境界に近い）
        for _ in 0..4 {
            lo = lo.wrapping_add(110);
            t.update(lo, 0);
        }
        // 次が +130（signed では -126 に見える）。直前の速度 110 に近い +130 のほうが正しい。
        let before = t.pos.0;
        lo = lo.wrapping_add(130);
        let (x, _) = t.update(lo, 0);
        assert_eq!(x as i32 - before, 130);
    }

    #[test]
    fn reset_returns_to_origin() {
        let mut t = LowByteTracker::new();
        t.update(0, 0);
        t.update(50, 50);
        t.reset();
        assert_eq!(t.update(7, 7), (ORIGIN as u16, ORIGIN as u16));
    }
}
