//! トラックパッドの絶対座標列を、マウスレポート（移動・クリック・ドラッグ・スクロール）に変換する。
//!
//! ハードウェアに依存しない純粋なロジック。`no_std`・依存なし・時刻は呼び出し側が与える。
//!
//! 使い方: 一定間隔（例: 5〜10ms）ごと、または新しいサンプルが届くたびに [`Pointer::update`] を呼ぶ。
//! 指が触れている間は最新の座標を `Some((x, y))`、離れていれば `None` を渡す。
//! 戻り値が `Some` のときだけ BLE にマウスレポートを送ればよい。
//!
//! 機能:
//! - 相対移動への変換（位置の平滑化、速度に応じた加速、端数の持ち越し）
//! - タップでクリック、タップ直後のタッチ&ホールドでドラッグ
//! - スクロールモード（キー押下中など）では移動をホイール/パンに変換

#![cfg_attr(not(test), no_std)]

/// マウスレポート（BLE HID の ID2 と同じ並び）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseReport {
    /// bit0: 左, bit1: 右, bit2: 中
    pub buttons: u8,
    pub x: i8,
    pub y: i8,
    pub wheel: i8,
    pub pan: i8,
}

impl MouseReport {
    pub const BUTTON_LEFT: u8 = 1;

    /// BLE の ID2 レポートの 5 バイト（buttons, x, y, wheel, pan）。
    pub fn to_bytes(self) -> [u8; 5] {
        [self.buttons, self.x as u8, self.y as u8, self.wheel as u8, self.pan as u8]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// 座標のクリップ範囲（Pinnacle の実用範囲）。
    pub x_min: u16,
    pub x_max: u16,
    pub y_min: u16,
    pub y_max: u16,
    /// 軸の反転。
    pub invert_x: bool,
    pub invert_y: bool,
    /// 移動の基本ゲイン（1/256 単位。256 で等倍）。
    pub gain_q8: u32,
    /// 加速: 1 サンプルあたりの移動量（座標単位）がこの値のとき、ゲインが 2 倍になる。0 で加速なし。
    pub accel_ref: u32,
    /// 加速の上限（ゲインの倍率の上限, 1/256 単位）。
    pub accel_max_q8: u32,
    /// 位置の平滑化係数（1/256 単位。256 = 平滑化なし、小さいほど滑らか・遅れる）。
    pub smoothing_q8: i32,
    /// タップと判定する最大接触時間 (ms)。
    pub tap_max_ms: u32,
    /// タップと判定する最大移動量（座標単位）。
    pub tap_max_move: i32,
    /// タップのクリックを押している時間 (ms)。
    pub click_hold_ms: u32,
    /// タップの指を離してから、この時間内の再タッチはドラッグ（ボタンを押したまま）にする (ms)。
    pub drag_window_ms: u32,
    /// スクロール 1 目盛りあたりの移動量（座標単位）。
    pub scroll_div: i32,
    /// スクロール方向の反転（Mac の「ナチュラルなスクロール」に合わせる用）。
    pub scroll_invert_y: bool,
    pub scroll_invert_x: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            x_min: 128,
            x_max: 1920,
            y_min: 64,
            y_max: 1472,
            invert_x: false,
            invert_y: false,
            gain_q8: 160,
            accel_ref: 40,
            accel_max_q8: 3 * 256,
            smoothing_q8: 192,
            tap_max_ms: 180,
            tap_max_move: 60,
            click_hold_ms: 20,
            drag_window_ms: 250,
            scroll_div: 40,
            scroll_invert_y: false,
            scroll_invert_x: false,
        }
    }
}

/// 1 軸ぶんの「端数の持ち越し」。Q8 固定小数点で合計し、整数部を出力して余りを残す。
#[derive(Clone, Copy, Debug, Default)]
struct Accum(i32);

impl Accum {
    /// `delta_q8` を加えて、`i8` の範囲で整数部を取り出す。はみ出した分と端数は残す。
    fn push(&mut self, delta_q8: i32) -> i8 {
        self.0 += delta_q8;
        let whole = (self.0 / 256).clamp(-127, 127);
        self.0 -= whole * 256;
        whole as i8
    }

    fn reset(&mut self) {
        self.0 = 0;
    }
}

#[derive(Clone, Copy, Debug)]
struct Touch {
    down_ms: u32,
    start_x: i32,
    start_y: i32,
    max_move: i32,
    /// この接触はドラッグ（ボタンを押し続ける）として始まった。
    dragging: bool,
    /// スクロールモードで操作した（タップ扱いにしない）。
    scrolled: bool,
}

pub struct Pointer {
    cfg: Config,
    /// 平滑化後の位置（Q8）。
    pos: Option<(i32, i32)>,
    touch: Option<Touch>,
    acc_x: Accum,
    acc_y: Accum,
    acc_wheel: Accum,
    acc_pan: Accum,
    /// 押しているボタン。
    buttons: u8,
    /// 最後に送ったボタン状態。
    sent_buttons: u8,
    /// タップのクリックを離す時刻。
    click_release_at: Option<u32>,
    /// 最後にタップ（指を離した）した時刻。ドラッグ判定用。
    last_tap_ms: Option<u32>,
}

impl Pointer {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            pos: None,
            touch: None,
            acc_x: Accum::default(),
            acc_y: Accum::default(),
            acc_wheel: Accum::default(),
            acc_pan: Accum::default(),
            buttons: 0,
            sent_buttons: 0,
            click_release_at: None,
            last_tap_ms: None,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    fn clip(&self, x: u16, y: u16) -> (i32, i32) {
        let c = &self.cfg;
        let mut x = x.clamp(c.x_min, c.x_max) as i32;
        let mut y = y.clamp(c.y_min, c.y_max) as i32;
        if c.invert_x {
            x = (c.x_min as i32 + c.x_max as i32) - x;
        }
        if c.invert_y {
            y = (c.y_min as i32 + c.y_max as i32) - y;
        }
        (x, y)
    }

    /// 状態を進める。変化（移動・スクロール・ボタン）があれば報告を返す。
    ///
    /// - `now_ms`: 単調増加する時刻（ms）
    /// - `touch`: 指が触れていれば最新の生座標、離れていれば `None`
    /// - `scroll_mode`: true の間は移動をスクロールに変換する
    pub fn update(&mut self, now_ms: u32, touch: Option<(u16, u16)>, scroll_mode: bool) -> Option<MouseReport> {
        let (mut dx, mut dy) = (0i32, 0i32);

        // タップのクリックを離す。
        if let Some(t) = self.click_release_at {
            if now_ms.wrapping_sub(t) < u32::MAX / 2 {
                self.buttons &= !MouseReport::BUTTON_LEFT;
                self.click_release_at = None;
            }
        }

        match touch {
            Some((rx, ry)) => {
                let (x, y) = self.clip(rx, ry);
                let (x_q8, y_q8) = (x << 8, y << 8);

                if self.touch.is_none() {
                    // 接触開始。位置は平滑化せずそのまま採用し、移動量は 0 から始める。
                    let dragging = self
                        .last_tap_ms
                        .is_some_and(|t| now_ms.wrapping_sub(t) <= self.cfg.drag_window_ms);
                    self.touch = Some(Touch {
                        down_ms: now_ms,
                        start_x: x,
                        start_y: y,
                        max_move: 0,
                        dragging,
                        scrolled: false,
                    });
                    self.pos = Some((x_q8, y_q8));
                    self.last_tap_ms = None;
                    if dragging {
                        // 保留中のクリック解放を取り消し、ボタンを押し続ける。
                        self.click_release_at = None;
                        self.buttons |= MouseReport::BUTTON_LEFT;
                    }
                } else if let (Some((px, py)), Some(t)) = (self.pos, self.touch.as_mut()) {
                    // 平滑化した新しい位置。
                    let a = self.cfg.smoothing_q8.clamp(1, 256);
                    let nx = px + (x_q8 - px) * a / 256;
                    let ny = py + (y_q8 - py) * a / 256;
                    // 移動量（Q8 の差を座標単位に見立てた値）。
                    let ddx = nx - px;
                    let ddy = ny - py;
                    self.pos = Some((nx, ny));
                    t.max_move = t.max_move.max((x - t.start_x).abs() + (y - t.start_y).abs());

                    if scroll_mode {
                        t.scrolled = true;
                        let sy = if self.cfg.scroll_invert_y { ddy } else { -ddy };
                        let sx = if self.cfg.scroll_invert_x { -ddx } else { ddx };
                        // 1 目盛り = scroll_div 座標単位 → Q8 の整数部へ
                        let div = self.cfg.scroll_div.max(1);
                        let wheel = self.acc_wheel.push(sy / div);
                        let pan = self.acc_pan.push(sx / div);
                        return self.finish(now_ms, 0, 0, wheel, pan);
                    }
                    dx = ddx;
                    dy = ddy;
                }
            }
            None => {
                if let Some(t) = self.touch.take() {
                    // 指が離れた。
                    let is_tap = !t.dragging
                        && !t.scrolled
                        && now_ms.wrapping_sub(t.down_ms) <= self.cfg.tap_max_ms
                        && t.max_move <= self.cfg.tap_max_move;
                    if t.dragging {
                        self.buttons &= !MouseReport::BUTTON_LEFT;
                        self.last_tap_ms = None;
                    } else if is_tap {
                        self.buttons |= MouseReport::BUTTON_LEFT;
                        self.click_release_at = Some(now_ms.wrapping_add(self.cfg.click_hold_ms));
                        self.last_tap_ms = Some(now_ms);
                    }
                    self.pos = None;
                    self.acc_x.reset();
                    self.acc_y.reset();
                    self.acc_wheel.reset();
                    self.acc_pan.reset();
                }
            }
        }

        // 相対移動（Q8 の差）にゲインと加速を掛ける。
        // 動いていなくても、i8 に収まらず持ち越した分があれば出し切るため毎回 push する。
        let speed = (dx.abs().max(dy.abs()) >> 8) as u32;
        let mut gain = self.cfg.gain_q8;
        if self.cfg.accel_ref > 0 {
            let boost = (gain * speed.min(self.cfg.accel_ref * 4) / self.cfg.accel_ref).min(self.cfg.accel_max_q8);
            gain += boost;
        }
        let ox = self.acc_x.push(dx * gain as i32 / 256);
        let oy = self.acc_y.push(dy * gain as i32 / 256);
        self.finish(now_ms, ox, oy, 0, 0)
    }

    fn finish(&mut self, _now_ms: u32, x: i8, y: i8, wheel: i8, pan: i8) -> Option<MouseReport> {
        if x == 0 && y == 0 && wheel == 0 && pan == 0 && self.buttons == self.sent_buttons {
            return None;
        }
        self.sent_buttons = self.buttons;
        Some(MouseReport { buttons: self.buttons, x, y, wheel, pan })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 加速・平滑化なしの素直な設定（計算を読みやすくする）。
    fn plain() -> Config {
        Config { gain_q8: 256, accel_ref: 0, smoothing_q8: 256, ..Config::default() }
    }

    fn run(p: &mut Pointer, steps: &[(u32, Option<(u16, u16)>)]) -> std::vec::Vec<MouseReport> {
        steps.iter().filter_map(|&(t, s)| p.update(t, s, false)).collect()
    }

    #[test]
    fn first_sample_does_not_jump() {
        let mut p = Pointer::new(plain());
        assert_eq!(p.update(0, Some((1000, 700)), false), None);
    }

    #[test]
    fn moves_by_delta_at_unity_gain() {
        let mut p = Pointer::new(plain());
        p.update(0, Some((1000, 700)), false);
        let r = p.update(10, Some((1010, 695)), false).unwrap();
        assert_eq!((r.x, r.y), (10, -5));
    }

    #[test]
    fn fractional_remainder_is_carried_over() {
        let cfg = Config { gain_q8: 128, ..plain() }; // 0.5 倍
        let mut p = Pointer::new(cfg);
        p.update(0, Some((1000, 700)), false);
        // 1 座標ずつ動くと 0.5 → 出力 0、次で合計 1.0 → 出力 1
        assert_eq!(p.update(10, Some((1001, 700)), false), None);
        let r = p.update(20, Some((1002, 700)), false).unwrap();
        assert_eq!(r.x, 1);
    }

    #[test]
    fn clips_to_active_range() {
        let mut p = Pointer::new(plain());
        p.update(0, Some((10, 10)), false); // 範囲外 → (128, 64) に丸める
        // 範囲外のままなら動きなし
        assert_eq!(p.update(10, Some((0, 0)), false), None);
        let r = p.update(20, Some((138, 74)), false).unwrap();
        assert_eq!((r.x, r.y), (10, 10));
    }

    #[test]
    fn invert_y_flips_direction() {
        let mut p = Pointer::new(Config { invert_y: true, ..plain() });
        p.update(0, Some((1000, 700)), false);
        let r = p.update(10, Some((1000, 710)), false).unwrap();
        assert_eq!(r.y, -10);
    }

    #[test]
    fn large_delta_is_clamped_and_remainder_kept() {
        let mut p = Pointer::new(plain());
        p.update(0, Some((200, 700)), false);
        let r = p.update(10, Some((500, 700)), false).unwrap(); // +300
        assert_eq!(r.x, 127);
        // 触れたまま同じ位置: 残り 173 のうち 127 を出す
        let r = p.update(20, Some((500, 700)), false).unwrap();
        assert_eq!(r.x, 127);
        let r = p.update(30, Some((500, 700)), false).unwrap();
        assert_eq!(r.x, 46);
        assert_eq!(p.update(40, Some((500, 700)), false), None);
    }

    #[test]
    fn acceleration_increases_gain_with_speed() {
        let cfg = Config { gain_q8: 256, accel_ref: 40, smoothing_q8: 256, ..Config::default() };
        let mut slow = Pointer::new(cfg);
        let mut fast = Pointer::new(cfg);
        slow.update(0, Some((200, 700)), false);
        fast.update(0, Some((200, 700)), false);
        let s = slow.update(10, Some((210, 700)), false).unwrap().x; // 10 単位
        let f = fast.update(10, Some((240, 700)), false).unwrap().x; // 40 単位 → ゲイン 2 倍
        assert_eq!(s, 12); // 10 * (1 + 10/40) = 12.5 → 12
        assert_eq!(f, 80);
        assert!(f > 40 * 1);
    }

    #[test]
    fn smoothing_lags_behind_raw_input() {
        let mut p = Pointer::new(Config { smoothing_q8: 128, ..plain() });
        p.update(0, Some((1000, 700)), false);
        let r = p.update(10, Some((1020, 700)), false).unwrap();
        assert_eq!(r.x, 10); // 半分だけ追従
    }

    #[test]
    fn quick_still_touch_is_a_tap_click() {
        let mut p = Pointer::new(plain());
        let reports = run(
            &mut p,
            &[
                (0, Some((1000, 700))),
                (50, Some((1002, 701))),
                (100, None), // 離す → クリック押下
                (110, None),
                (125, None), // click_hold_ms=20 経過 → 解放
            ],
        );
        assert!(reports.len() >= 2, "{reports:?}");
        let down = reports.iter().find(|r| r.buttons == 1).expect("press");
        assert_eq!((down.x, down.y), (0, 0));
        let last = reports.last().unwrap();
        assert_eq!(last.buttons, 0, "release");
    }

    #[test]
    fn long_touch_is_not_a_tap() {
        let mut p = Pointer::new(plain());
        let reports = run(&mut p, &[(0, Some((1000, 700))), (300, Some((1000, 700))), (400, None), (500, None)]);
        assert!(reports.iter().all(|r| r.buttons == 0), "{reports:?}");
    }

    #[test]
    fn moved_touch_is_not_a_tap() {
        let mut p = Pointer::new(plain());
        let reports = run(&mut p, &[(0, Some((1000, 700))), (30, Some((1100, 700))), (60, None), (100, None)]);
        assert!(reports.iter().all(|r| r.buttons == 0), "{reports:?}");
    }

    #[test]
    fn tap_then_touch_and_hold_drags() {
        let mut p = Pointer::new(plain());
        // タップ
        run(&mut p, &[(0, Some((1000, 700))), (40, None), (70, None)]);
        // 250ms 以内に再タッチ → ボタンを押したまま移動
        let a = p.update(150, Some((1000, 700)), false).expect("press at second touch");
        assert_eq!(a.buttons, 1);
        let b = p.update(170, Some((1030, 700)), false).unwrap();
        assert_eq!((b.buttons, b.x), (1, 30));
        // 離すとボタンを離す
        let c = p.update(300, None, false).unwrap();
        assert_eq!(c.buttons, 0);
    }

    #[test]
    fn second_touch_after_window_is_a_normal_move() {
        let mut p = Pointer::new(plain());
        run(&mut p, &[(0, Some((1000, 700))), (40, None), (70, None)]);
        assert_eq!(p.update(1000, Some((1000, 700)), false), None);
        let r = p.update(1010, Some((1010, 700)), false).unwrap();
        assert_eq!(r.buttons, 0);
    }

    #[test]
    fn scroll_mode_converts_motion_to_wheel() {
        let mut p = Pointer::new(plain()); // scroll_div = 40
        p.update(0, Some((1000, 700)), true);
        // 指を下へ 80 → 上スクロール方向（既定: 下に動かすと負）
        let r = p.update(10, Some((1000, 780)), true).unwrap();
        assert_eq!((r.x, r.y), (0, 0));
        assert_eq!(r.wheel, -2);
        // 横
        let r = p.update(20, Some((1080, 780)), true).unwrap();
        assert_eq!(r.pan, 2);
    }

    #[test]
    fn scroll_invert_flips_wheel() {
        let mut p = Pointer::new(Config { scroll_invert_y: true, ..plain() });
        p.update(0, Some((1000, 700)), true);
        let r = p.update(10, Some((1000, 780)), true).unwrap();
        assert_eq!(r.wheel, 2);
    }

    #[test]
    fn scroll_gesture_is_never_a_tap() {
        let mut p = Pointer::new(plain());
        let steps = [(0u32, Some((1000u16, 700u16))), (20, Some((1000, 710)))];
        for (t, s) in steps {
            p.update(t, s, true);
        }
        let reports: std::vec::Vec<_> = [(40u32, None), (70, None)].into_iter().filter_map(|(t, s)| p.update(t, s, true)).collect();
        assert!(reports.iter().all(|r| r.buttons == 0), "{reports:?}");
    }

    #[test]
    fn lifting_resets_motion_so_next_touch_does_not_jump() {
        let mut p = Pointer::new(plain());
        p.update(0, Some((300, 300)), false);
        p.update(400, Some((300, 300)), false); // 長押し（タップにしない）
        p.update(410, None, false);
        assert_eq!(p.update(900, Some((1500, 1000)), false), None);
    }

    #[test]
    fn to_bytes_matches_ble_report_layout() {
        let r = MouseReport { buttons: 1, x: -1, y: 2, wheel: -3, pan: 4 };
        assert_eq!(r.to_bytes(), [1, 0xFF, 2, 0xFD, 4]);
    }
}
