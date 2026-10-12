//! 相対モード（パッド自身が計算した移動量）→ BLE のマウスレポート（ID2 の 5 バイト）。
//!
//! `firmware/hikyaku-s3/src/bin/trackpad/mod.rs` の相対モードの変換（軸の入れ替え・反転・倍率・端数の持ち越し・
//! レポートの分割）を、ホストでテストできるように切り出したもの。**出力はファームの元の処理と 1 ビットも変わらない**
//! （`tests` の `golden` は元のコードの写しで、実機ログと乱数の入力で突き合わせている）。
//!
//! 使い方: パケットが届くたびに [`Relay::push`]、届かなかった周期には [`Relay::poll`] を呼ぶ。
//! 出すべきレポートがあれば、渡したクロージャが 5 バイト（buttons, x, y, wheel, pan）で呼ばれる。
//!
//! 実機ログ（`trackpad-trace` の `rel <ms> raw=[b0, b1, b2]` 行）は [`parse_trace_line`] で読める
//! （`cargo run -p hikyaku-pointer --example replay_rel -- tools/traces/<log>`）。

use hikyaku_pinnacle::{parse_relative, Relative};

/// 相対モードの変換の設定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// パッドの X/Y を入れ替える（パッドが 90 度回った向きで付いているとき）。
    pub swap_xy: bool,
    /// 画面の X を反転する（入れ替えの後に適用）。
    pub invert_x: bool,
    /// 画面の Y を反転する（入れ替えの後に適用）。
    pub invert_y: bool,
    /// 移動量の倍率（整数倍）。
    pub gain: i32,
    /// スクロールモードで、ホイール 1 目盛りに相当する移動量。
    pub scroll_div: i32,
    /// 静止中のぶれを抑えるフィルタ（[`StillFilter`]）。`None` で無効（ファームの既定）。
    pub still: Option<StillFilter>,
}

impl Config {
    /// ファーム（`trackpad/mod.rs`）の 2026-10-13 時点の既定値。持ち主が実機で「調子が良い」と確認した設定。
    pub const FIRMWARE_DEFAULT: Config =
        Config { swap_xy: true, invert_x: true, invert_y: true, gain: 2, scroll_div: 8, still: None };
}

/// 静止中のぶれを抑えるフィルタの設定（ファームでは `trackpad-still-filter` feature）。
///
/// 考え方: 指を止めているときのぶれは、パッドが報告する**位置**が ±1〜2 揺れているもので、移動量の合計は増えない
/// （行って戻る）。そこで「静止」状態では移動量を足し込むだけにして、合計が `threshold` に届いたときに初めて、
/// 溜めた分をまとめて出す（動きは失わない）。1 パケットで `threshold` 以上動いたら、溜めずにそのまま通す
/// （スッと触ったときの反応を鈍らせない）。一度動き出したら「移動」状態になり、以降は 1:1 で通す
/// （ゆっくり動かしても刻まない）。直近 [`STILL_WINDOW`] パケットの移動量の合計が `threshold` 未満になったら静止に戻す。
/// パケットが `gap_ms` 途切れたら（指を離した）、静止に戻して溜めた分を捨てる。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StillFilter {
    /// 静止とみなす移動量の上限（パッドの単位。相対モードの 1 単位はおよそ 0.15〜0.2mm）。
    pub threshold: i32,
    /// パケットがこの時間（ms）途切れたら、指が離れたとみなす。
    pub gap_ms: u32,
}

impl StillFilter {
    pub const DEFAULT: StillFilter = StillFilter { threshold: 3, gap_ms: 100 };
}

impl Default for StillFilter {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 「移動」から「静止」に戻る判定に使う、直近のパケット数（パッドは約 100Hz なので約 80ms）。
pub const STILL_WINDOW: usize = 8;

#[derive(Clone, Copy, Debug, Default)]
struct StillState {
    moving: bool,
    /// 静止中に溜めた移動量。
    acc_x: i32,
    acc_y: i32,
    /// 移動中の直近の移動量（リングバッファ）と、その書き込み位置。
    hist: [(i32, i32); STILL_WINDOW],
    hist_pos: usize,
    hist_len: usize,
    last_ms: Option<u32>,
}

impl StillState {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn apply(&mut self, f: &StillFilter, now_ms: u32, sx: i32, sy: i32) -> (i32, i32) {
        if let Some(last) = self.last_ms {
            if now_ms.wrapping_sub(last) > f.gap_ms {
                self.reset();
            }
        }
        self.last_ms = Some(now_ms);
        if self.moving {
            self.record(sx, sy);
            if self.hist_len == STILL_WINDOW {
                let (wx, wy) = self.hist.iter().fold((0, 0), |a, d| (a.0 + d.0, a.1 + d.1));
                if wx.abs() < f.threshold && wy.abs() < f.threshold {
                    self.moving = false;
                    self.acc_x = 0;
                    self.acc_y = 0;
                }
            }
            return (sx, sy);
        }
        // 静止中: 溜める。1 パケットで大きく動いたか、溜めた分がしきい値に届いたら、まとめて出して「移動」へ。
        self.acc_x += sx;
        self.acc_y += sy;
        let big = sx.abs() >= f.threshold || sy.abs() >= f.threshold;
        if big || self.acc_x.abs() >= f.threshold || self.acc_y.abs() >= f.threshold {
            let out = (self.acc_x, self.acc_y);
            self.acc_x = 0;
            self.acc_y = 0;
            self.moving = true;
            self.hist_len = 0;
            self.hist_pos = 0;
            self.record(out.0, out.1);
            return out;
        }
        (0, 0)
    }

    fn record(&mut self, sx: i32, sy: i32) {
        self.hist[self.hist_pos] = (sx, sy);
        self.hist_pos = (self.hist_pos + 1) % STILL_WINDOW;
        if self.hist_len < STILL_WINDOW {
            self.hist_len += 1;
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::FIRMWARE_DEFAULT
    }
}

/// 相対モードの変換器。パッドのパケットを受け取り、マウスレポートを出す。
#[derive(Clone, Debug)]
pub struct Relay {
    cfg: Config,
    // i8 に収まらない分は次に持ち越す
    carry_x: i32,
    carry_y: i32,
    carry_wheel: i32,
    /// 最後に送ったボタン状態。
    last_buttons: u8,
    still: StillState,
}

impl Relay {
    pub const fn new(cfg: Config) -> Self {
        Self {
            cfg,
            carry_x: 0,
            carry_y: 0,
            carry_wheel: 0,
            last_buttons: 0,
            still: StillState {
                moving: false,
                acc_x: 0,
                acc_y: 0,
                hist: [(0, 0); STILL_WINDOW],
                hist_pos: 0,
                hist_len: 0,
                last_ms: None,
            },
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// パッドの座標系の移動量を、画面の座標系に直す（入れ替え → 反転）。
    pub fn orient(&self, dx: i16, dy: i16) -> (i32, i32) {
        let (mut sx, mut sy) = if self.cfg.swap_xy { (dy as i32, dx as i32) } else { (dx as i32, dy as i32) };
        if self.cfg.invert_x {
            sx = -sx;
        }
        if self.cfg.invert_y {
            sy = -sy;
        }
        (sx, sy)
    }

    /// パケットを 1 つ処理する。出すレポートごとに `emit` を呼ぶ。
    ///
    /// - `now_ms`: 単調増加する時刻（ms）。時間を使う処理（フィルタ・タップ判定）のため。基本の変換では使わない。
    /// - `scroll_mode`: true の間は移動をホイールに変換する。
    pub fn push(&mut self, now_ms: u32, pkt: &Relative, scroll_mode: bool, emit: impl FnMut([u8; 5])) {
        let buttons = pkt.buttons & 1; // タップ = 左クリック
        let (mut sx, mut sy) = self.orient(pkt.dx, pkt.dy);
        if let Some(f) = self.cfg.still {
            (sx, sy) = self.still.apply(&f, now_ms, sx, sy);
        }
        self.accumulate(sx, sy, scroll_mode);
        self.flush(buttons, emit);
    }

    /// パケットが無かった周期に呼ぶ。時間で決まる処理（タップのクリック解放など）のため。基本の変換では何もしない。
    pub fn poll(&mut self, now_ms: u32, emit: impl FnMut([u8; 5])) {
        let _ = (now_ms, emit);
    }

    fn accumulate(&mut self, sx: i32, sy: i32, scroll_mode: bool) {
        if scroll_mode {
            self.carry_wheel += -sy;
        } else {
            self.carry_x += sx * self.cfg.gain;
            self.carry_y += sy * self.cfg.gain;
        }
    }

    /// 持ち越し分とボタン状態から、レポートを送り切る（大きいときは 127 ずつ分けて送る）。
    fn flush(&mut self, buttons: u8, mut emit: impl FnMut([u8; 5])) {
        let div = self.cfg.scroll_div;
        let wheel = (self.carry_wheel / div).clamp(-127, 127);
        self.carry_wheel -= wheel * div;
        loop {
            let x = self.carry_x.clamp(-127, 127);
            let y = self.carry_y.clamp(-127, 127);
            if x == 0 && y == 0 && wheel == 0 && buttons == self.last_buttons {
                break;
            }
            self.carry_x -= x;
            self.carry_y -= y;
            // 注: 移動とホイールが同じパケットで出たときは、ホイールは移動のレポートに乗らず、持ち越しも消える
            // （元のファームの挙動。スクロールモードの切り替わり目でしか起きない）。
            let w = if wheel != 0 && x == 0 && y == 0 { wheel } else { 0 };
            emit([buttons, x as i8 as u8, y as i8 as u8, w as i8 as u8, 0]);
            self.last_buttons = buttons;
            if self.carry_x == 0 && self.carry_y == 0 {
                break;
            }
        }
    }
}

/// 実機ログの 1 行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceLine {
    /// ファームの `Instant::now().as_millis()`。
    pub ms: u32,
    pub raw: [u8; 3],
}

impl TraceLine {
    pub fn packet(&self) -> Relative {
        parse_relative(&self.raw)
    }
}

/// `trackpad-trace` のログの `rel <ms> raw=[b0, b1, b2] dx=.. dy=..` を読む。
/// 該当しない行や、途中で化けている行（シリアルの取りこぼし）は `None`。`no_std` で動く（確保しない）。
pub fn parse_trace_line(line: &str) -> Option<TraceLine> {
    let i = line.find("rel ")?;
    let rest = &line[i + 4..];
    let (ms_str, rest) = rest.split_once(' ')?;
    let ms: u32 = ms_str.parse().ok()?;
    let rest = rest.strip_prefix("raw=[")?;
    let (inner, _) = rest.split_once(']')?;
    let mut raw = [0u8; 3];
    let mut parts = inner.split(',');
    for b in raw.iter_mut() {
        let p = parts.next()?.trim();
        if p.len() != 2 {
            return None;
        }
        *b = u8::from_str_radix(p, 16).ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(TraceLine { ms, raw })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn pkt(buttons: u8, dx: i16, dy: i16) -> Relative {
        Relative { buttons, dx, dy, raw: [0; 3] }
    }

    fn run(relay: &mut Relay, pkts: &[(Relative, bool)]) -> Vec<[u8; 5]> {
        let mut out = Vec::new();
        for (i, (p, scroll)) in pkts.iter().enumerate() {
            relay.push(i as u32 * 10, p, *scroll, |r| out.push(r));
        }
        out
    }

    fn plain() -> Config {
        Config { swap_xy: false, invert_x: false, invert_y: false, gain: 1, scroll_div: 8, still: None }
    }

    fn with_still() -> Config {
        Config { still: Some(StillFilter::DEFAULT), ..plain() }
    }

    /// 10ms 間隔のパケット列を流して、出た移動量（x, y）の列を返す。
    fn run_moves(relay: &mut Relay, deltas: &[(i16, i16)]) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for (i, &(dx, dy)) in deltas.iter().enumerate() {
            relay.push(i as u32 * 10, &pkt(0, dx, dy), false, |r| out.push((r[1] as i8 as i32, r[2] as i8 as i32)));
        }
        out
    }

    #[test]
    fn still_filter_suppresses_resting_jitter() {
        let mut r = Relay::new(with_still());
        // 指を置いたまま: ±1〜2 が行って戻る
        let jitter = [(1, 0), (-1, 1), (0, -1), (1, 1), (-2, 0), (1, -1), (0, 1), (-1, 0), (2, -1), (-1, 1), (0, 0), (-1, 0)];
        assert!(run_moves(&mut r, &jitter).is_empty());
    }

    #[test]
    fn still_filter_passes_a_quick_touch_through_unchanged() {
        let mut r = Relay::new(with_still());
        // スッと触る: 最初のパケットから大きい
        assert_eq!(run_moves(&mut r, &[(6, -4), (9, -7), (3, -2)]), vec![(6, -4), (9, -7), (3, -2)]);
    }

    #[test]
    fn still_filter_keeps_slow_motion_without_losing_distance() {
        let mut r = Relay::new(with_still());
        // 毎パケット 1 ずつ、ゆっくり右へ
        let out = run_moves(&mut r, &[(1, 0); 12]);
        // 最初はしきい値（3）まで溜めてまとめて出し、以降は 1:1
        assert_eq!(out[0], (3, 0));
        assert!(out[1..].iter().all(|&d| d == (1, 0)));
        assert_eq!(out.iter().map(|d| d.0).sum::<i32>(), 12);
    }

    #[test]
    fn still_filter_small_first_packet_is_not_lost_when_motion_follows() {
        let mut r = Relay::new(with_still());
        // 置いた瞬間の小さい動き → 直後に大きい動き: 合計が保たれる
        assert_eq!(run_moves(&mut r, &[(1, -1), (5, 0)]), vec![(6, -1)]);
    }

    #[test]
    fn still_filter_returns_to_still_after_motion_stops() {
        let mut r = Relay::new(with_still());
        let mut seq: Vec<(i16, i16)> = vec![(5, 5); 5]; // 動く
        seq.extend([(1, 0), (-1, 0), (0, 1), (0, -1), (1, -1), (-1, 1), (1, 0), (-1, 0)]); // 止まる（窓いっぱい）
        let tail: Vec<(i16, i16)> = vec![(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1)]; // 静止中のぶれ
        seq.extend(&tail);
        let out = run_moves(&mut r, &seq);
        // 動いている間と、止まった直後の窓ぶんは 1:1 で通る。その後のぶれは出ない。
        assert_eq!(out.len(), 5 + 8);
    }

    #[test]
    fn still_filter_gap_resets_accumulation() {
        let mut r = Relay::new(with_still());
        let mut out = Vec::new();
        r.push(0, &pkt(0, 2, 0), false, |x| out.push(x));
        r.push(10, &pkt(0, 0, 2), false, |x| out.push(x));
        // 指を離して（200ms 空いて）置き直す: 前の溜め（2, 2）は捨てられる
        r.push(300, &pkt(0, 2, 0), false, |x| out.push(x));
        assert!(out.is_empty());
        r.push(310, &pkt(0, 1, 0), false, |x| out.push(x));
        assert_eq!(out, vec![[0, 3, 0, 0, 0]]);
    }

    #[test]
    fn still_filter_conserves_distance_up_to_residual() {
        // 任意の列で、出した合計 = 入れた合計 − 溜め残し（|残し| < threshold）
        let mut rng = Lcg(7);
        for _ in 0..100 {
            let mut r = Relay::new(with_still());
            let (mut in_x, mut in_y, mut out_x, mut out_y) = (0i32, 0i32, 0i32, 0i32);
            for i in 0..300u32 {
                let d = |rng: &mut Lcg| (rng.next() % 9) as i16 - 4;
                let (dx, dy) = (d(&mut rng), d(&mut rng));
                in_x += dx as i32;
                in_y += dy as i32;
                r.push(i * 10, &pkt(0, dx, dy), false, |o| {
                    out_x += o[1] as i8 as i32;
                    out_y += o[2] as i8 as i32;
                });
            }
            let (rx, ry) = (in_x - out_x, in_y - out_y);
            assert!(rx.abs() < 3 && ry.abs() < 3, "residual ({rx}, {ry})");
        }
    }

    #[test]
    fn identity_passes_deltas_through() {
        let mut r = Relay::new(plain());
        let out = run(&mut r, &[(pkt(0, 3, -2), false)]);
        assert_eq!(out, vec![[0, 3, (-2i8) as u8, 0, 0]]);
    }

    #[test]
    fn no_report_without_change() {
        let mut r = Relay::new(plain());
        assert!(run(&mut r, &[(pkt(0, 0, 0), false)]).is_empty());
    }

    #[test]
    fn firmware_default_orientation() {
        // パッドの dx が負 = 画面の下（SWAP → sy = dx → 反転）
        let mut r = Relay::new(Config::FIRMWARE_DEFAULT);
        let out = run(&mut r, &[(pkt(0, -5, 0), false)]);
        assert_eq!(out, vec![[0, 0, 10, 0, 0]]); // gain 2
        let out = run(&mut r, &[(pkt(0, 0, 7), false)]);
        assert_eq!(out, vec![[0, (-14i8) as u8, 0, 0, 0]]);
    }

    #[test]
    fn large_motion_is_split_into_i8_chunks() {
        let mut r = Relay::new(Config { gain: 2, ..plain() });
        let out = run(&mut r, &[(pkt(0, 200, -130), false)]);
        // 400 → 127+127+127+19、-260 → -127-127-6
        assert_eq!(
            out,
            vec![[0, 127, (-127i8) as u8, 0, 0], [0, 127, (-127i8) as u8, 0, 0], [0, 127, (-6i8) as u8, 0, 0], [0, 19, 0, 0, 0]]
        );
    }

    #[test]
    fn button_change_alone_makes_a_report() {
        let mut r = Relay::new(plain());
        let out = run(&mut r, &[(pkt(1, 0, 0), false), (pkt(1, 0, 0), false), (pkt(0, 0, 0), false)]);
        assert_eq!(out, vec![[1, 0, 0, 0, 0], [0, 0, 0, 0, 0]]);
    }

    #[test]
    fn scroll_mode_accumulates_wheel() {
        let mut r = Relay::new(plain());
        // 画面の下向き（sy > 0）はホイール負。8 で 1 目盛り。
        let out = run(&mut r, &[(pkt(0, 0, 5), true), (pkt(0, 0, 5), true)]);
        assert_eq!(out, vec![[0, 0, 0, (-1i8) as u8, 0]]);
        // 残り 2 の持ち越し
        let out = run(&mut r, &[(pkt(0, 0, 6), true)]);
        assert_eq!(out, vec![[0, 0, 0, (-1i8) as u8, 0]]);
    }

    #[test]
    fn parse_trace_line_accepts_firmware_format() {
        let l = parse_trace_line("INFO - rel 26111 raw=[fc, c0, 00] dx=-64 dy=-256").unwrap();
        assert_eq!(l, TraceLine { ms: 26111, raw: [0xfc, 0xc0, 0x00] });
        assert_eq!((l.packet().dx, l.packet().dy), (-64, -256));
        // 先頭に uart_cap の経過秒が付いていてもよい
        assert!(parse_trace_line("541.1 INFO - rel 9283 raw=[00, 00, 00] dx=0 dy=0").is_some());
        // 化けた行
        assert_eq!(parse_trace_line("INFO - rel 27407 raw=[fc237.0 , fc, fc] dx=-4 dy=-4"), None);
        assert_eq!(parse_trace_line("INFO - rel 387"), None);
        assert_eq!(parse_trace_line("INFO - trackpad: packets=3"), None);
    }

    /// `firmware/hikyaku-s3/src/bin/trackpad/mod.rs`（2026-10-13, commit 82ea5c0）の相対モードのループの写し。
    /// **ここは変えないこと。** `Relay` がこれと同じ出力を出すことを確かめるための基準。
    struct Golden {
        carry_x: i32,
        carry_y: i32,
        carry_wheel: i32,
        last_buttons: u8,
    }

    const SWAP_XY: bool = true;
    const SCREEN_X_INVERT: bool = true;
    const SCREEN_Y_INVERT: bool = true;
    const REL_GAIN: i32 = 2;
    const SCROLL_DIV: i32 = 8;

    impl Golden {
        fn new() -> Self {
            Self { carry_x: 0, carry_y: 0, carry_wheel: 0, last_buttons: 0 }
        }

        fn step(&mut self, r: &Relative, scroll_mode: bool, out: &mut Vec<[u8; 5]>) {
            let buttons = r.buttons & 1; // タップ = 左クリック
            let (mut sx, mut sy) = if SWAP_XY { (r.dy as i32, r.dx as i32) } else { (r.dx as i32, r.dy as i32) };
            if SCREEN_X_INVERT {
                sx = -sx;
            }
            if SCREEN_Y_INVERT {
                sy = -sy;
            }
            if scroll_mode {
                self.carry_wheel += -sy;
            } else {
                self.carry_x += sx * REL_GAIN;
                self.carry_y += sy * REL_GAIN;
            }
            let wheel = (self.carry_wheel / SCROLL_DIV).clamp(-127, 127);
            self.carry_wheel -= wheel * SCROLL_DIV;
            // 1 パケットぶんを送り切る（大きいときは 127 ずつ分けて送る）
            loop {
                let x = self.carry_x.clamp(-127, 127);
                let y = self.carry_y.clamp(-127, 127);
                if x == 0 && y == 0 && wheel == 0 && buttons == self.last_buttons {
                    break;
                }
                self.carry_x -= x;
                self.carry_y -= y;
                let w = if wheel != 0 && x == 0 && y == 0 { wheel } else { 0 };
                out.push([buttons, x as i8 as u8, y as i8 as u8, w as i8 as u8, 0]);
                self.last_buttons = buttons;
                if self.carry_x == 0 && self.carry_y == 0 {
                    break;
                }
            }
        }
    }

    /// 簡単な擬似乱数（依存を増やさない）。
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32
        }
    }

    #[test]
    fn matches_firmware_on_random_packets() {
        let mut rng = Lcg(20261013);
        for _ in 0..200 {
            let mut relay = Relay::new(Config::FIRMWARE_DEFAULT);
            let mut golden = Golden::new();
            let (mut got, mut want) = (Vec::new(), Vec::new());
            let mut scroll = false;
            for i in 0..500u32 {
                if rng.next().is_multiple_of(50) {
                    scroll = !scroll;
                }
                let raw = [(rng.next() & 0x3F) as u8, rng.next() as u8, rng.next() as u8];
                let p = parse_relative(&raw);
                golden.step(&p, scroll, &mut want);
                relay.push(i * 10, &p, scroll, |r| got.push(r));
            }
            assert_eq!(got, want);
        }
    }

    #[test]
    fn matches_firmware_on_recorded_trace() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/traces/2026-10-13-relative-all.log");
        let text = std::fs::read_to_string(path).expect("trace log in tools/traces");
        let mut relay = Relay::new(Config::FIRMWARE_DEFAULT);
        let mut golden = Golden::new();
        let (mut got, mut want) = (Vec::new(), Vec::new());
        let mut lines = 0usize;
        for line in text.lines() {
            let Some(l) = parse_trace_line(line) else { continue };
            lines += 1;
            let p = l.packet();
            golden.step(&p, false, &mut want);
            relay.push(l.ms, &p, false, |r| got.push(r));
        }
        assert!(lines > 10_000, "trace should contain many packets, got {lines}");
        assert!(!want.is_empty());
        assert_eq!(got, want);
    }
}
