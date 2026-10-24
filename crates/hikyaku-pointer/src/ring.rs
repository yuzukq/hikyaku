//! ショートカットリング（docs/design/ring.md）: 絶対座標 → リングの角度・中心からの距離、と BLE で送るイベントの形式。
//!
//! 角度は**画面上の向き**で、0° = 上、時計回りに増える（90° = 右）。パッドが 90° 回って付いている分は、
//! カーソルと同じ向きの補正（`swap_xy`・`invert_x`・`invert_y`）で合わせる。
//! 距離は、パッドの円の半径を 1000 とした値（中心 = 0、縁 ≒ 1000。外へはみ出すと 1000 を超える）。
//!
//! `no_std` で浮動小数の数学関数（atan2・sqrt）が使えないので、近似と整数の平方根で計算する（角度の誤差は 0.1° 未満）。

use crate::Rotation;

/// パッドの円の位置と、画面の向きへの補正。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// 円の中心（パッドの座標）。
    pub cx: i32,
    pub cy: i32,
    /// 円の半径（パッドの座標。X と Y で分解能が違うので別々に持つ）。
    pub rx: i32,
    pub ry: i32,
    /// 画面の向きへの補正（カーソルの変換と同じ意味）。
    pub swap_xy: bool,
    pub invert_x: bool,
    pub invert_y: bool,
    /// 本体の置き方（角度に足す）。
    pub rotation: Rotation,
}

impl Geometry {
    /// 中心と半径は実測（2026-10-21、縁をなぞった範囲: X 260〜1719・Y 191〜1398）。
    /// 向き: X と Y を入れ替え、上下だけ反転する（2026-10-21 実機で確認）。カーソル（相対モード）は左右も反転しているが、
    /// 絶対座標では左右の向きが逆に出るので、左右は反転しない。
    pub const DEFAULT: Geometry =
        Geometry { cx: 989, cy: 794, rx: 729, ry: 603, swap_xy: true, invert_x: false, invert_y: true, rotation: Rotation::R0 };
}

/// パッドの座標 → （角度 0.01° 単位 0〜35999、距離 0〜、半径 = 1000）。
pub fn polar(g: &Geometry, x: u16, y: u16) -> (u16, u16) {
    // 中心からの位置を、半径 = 1000 に正規化する
    let nx = (x as i32 - g.cx) * 1000 / g.rx.max(1);
    let ny = (y as i32 - g.cy) * 1000 / g.ry.max(1);
    let (mut sx, mut sy) = if g.swap_xy { (ny, nx) } else { (nx, ny) };
    if g.invert_x {
        sx = -sx;
    }
    if g.invert_y {
        sy = -sy;
    }
    // 画面: sx は右が +、sy は下が +。0° = 上、時計回り → atan2(sx, -sy)
    let deg = atan2_deg(sx as f32, -sy as f32);
    let cdeg = ((deg * 100.0) as i32 + g.rotation.quarters() as i32 * 9000).rem_euclid(36000) as u16;
    let r = isqrt((sx * sx + sy * sy) as u32).min(u16::MAX as u32) as u16;
    (cdeg, r)
}

/// atan2(y, x) を度で返す（-180〜180）。最大誤差 約 0.09°。
fn atan2_deg(y: f32, x: f32) -> f32 {
    const PI: f32 = core::f32::consts::PI;
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let (ax, ay) = (if x < 0.0 { -x } else { x }, if y < 0.0 { -y } else { y });
    // atan(z) ≈ (π/4)z − z(|z|−1)(0.2447 + 0.0663|z|)（0 ≤ z ≤ 1）
    let atan01 = |z: f32| PI / 4.0 * z - z * (z - 1.0) * (0.2447 + 0.0663 * z);
    let a = if ay <= ax { atan01(ay / ax) } else { PI / 2.0 - atan01(ax / ay) };
    let a = match (x < 0.0, y < 0.0) {
        (false, false) => a,
        (true, false) => PI - a,
        (true, true) => -(PI - a),
        (false, true) => -a,
    };
    a * 180.0 / PI
}

fn isqrt(n: u32) -> u32 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// BLE の独自サービス（ring.state）で送るイベント。12 バイト固定。
///
/// | バイト | 内容 |
/// |---|---|
/// | 0 | 種類: 1 = 開いた、2 = 閉じた、3 = 位置、4 = 設定 |
/// | 1 | フラグ: bit0 = 触れている（設定のときは、本体の置き方 = 時計回りに 90° を何回か） |
/// | 2〜3 | （設定のときは）今の送り先（0 = 1 台目）、その送り先でスクロールを反転しているか |
/// | 2〜3 | 角度（0.01° 単位、0 = 上・時計回り。リトルエンディアン） |
/// | 4〜5 | 中心からの距離（半径 = 1000） |
/// | 6〜7 | パッドの X（生の値。調整用） |
/// | 8〜9 | パッドの Y（生の値） |
/// | 10 | Z（接触の強さ） |
/// | 11 | 通し番号（取りこぼしの確認用） |
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Opened,
    Closed,
    Position { touching: bool, angle_cdeg: u16, radius: u16, x: u16, y: u16, z: u8 },
    /// 今の設定（HELLO・設定の問い合わせ・変更への返事）。profile = 今の送り先（0 = 1 台目）、
    /// scroll_invert = その送り先でスクロールを反転しているか。
    Settings { rotation: Rotation, profile: u8, scroll_invert: bool },
}

pub const EVENT_LEN: usize = 12;

impl Event {
    pub fn encode(&self, seq: u8) -> [u8; EVENT_LEN] {
        let mut b = [0u8; EVENT_LEN];
        b[11] = seq;
        match *self {
            Event::Opened => b[0] = 1,
            Event::Closed => b[0] = 2,
            Event::Position { touching, angle_cdeg, radius, x, y, z } => {
                b[0] = 3;
                b[1] = touching as u8;
                b[2..4].copy_from_slice(&angle_cdeg.to_le_bytes());
                b[4..6].copy_from_slice(&radius.to_le_bytes());
                b[6..8].copy_from_slice(&x.to_le_bytes());
                b[8..10].copy_from_slice(&y.to_le_bytes());
                b[10] = z;
            }
            Event::Settings { rotation, profile, scroll_invert } => {
                b[0] = 4;
                b[1] = rotation.quarters();
                b[2] = profile;
                b[3] = scroll_invert as u8;
            }
        }
        b
    }
}

/// 送り先（プロファイル）の数。docs/design/multi-host.md
pub const PROFILES: usize = 2;

/// アプリ → hikyaku（ring.ctrl への書き込み）の 1 バイト目。
pub mod ctrl {
    /// アプリが接続した（これを受け取るまで F23 でリングを開かない）。`[0x00]`
    pub const HELLO: u8 = 0x00;
    /// リングを閉じる（項目を実行した後・キャンセル・ノブモードの終了）。`[0x01]`
    pub const CLOSE: u8 = 0x01;
    /// リングを開く（アプリのボタンや試験用。F23 と同じ）。`[0x02]`
    pub const OPEN: u8 = 0x02;
    /// キーの組み合わせを 1 回押して離す（本物のキーボード入力として Mac に届く）。`[0x10, 修飾キー, キーコード]`
    /// 修飾キーはブートキーボードと同じビット（bit0 LCtrl, 1 LShift, 2 LAlt(Option), 3 LGui(Command), 4〜7 は右側）。
    /// キーコードは HID の Usage ID（0 なら修飾キーだけ）。
    pub const KEY: u8 = 0x10;
    /// メディアキー（Consumer の Usage）を 1 回押して離す。`[0x11, 下位, 上位]`（リトルエンディアンの 16 ビット）
    pub const CONSUMER: u8 = 0x11;
    /// 本体の置き方を変える（フラッシュに保存する）。`[0x20, 時計回りに 90° を何回か（0〜3）]`。返事は設定のイベント
    pub const SET_ROTATION: u8 = 0x20;
    /// 今の設定を問い合わせる。`[0x21]`。返事は設定のイベント
    pub const GET_SETTINGS: u8 = 0x21;
    /// 今の送り先でスクロールを反転するか（フラッシュに保存）。`[0x22, 0 | 1]`。返事は設定のイベント
    pub const SET_SCROLL_INVERT: u8 = 0x22;
    /// 送り先（プロファイル）を切り替える。`[0x23, 0 | 1]`。hikyaku は保存して再起動し、その ID でつなぎ直す（今の接続は切れる）
    pub const SWITCH_PROFILE: u8 = 0x23;
}

/// よく使うメディアキー（HID Consumer Page の Usage ID）。
pub mod consumer {
    pub const VOLUME_UP: u16 = 0x00E9;
    pub const VOLUME_DOWN: u16 = 0x00EA;
    pub const MUTE: u16 = 0x00E2;
    pub const BRIGHTNESS_UP: u16 = 0x006F;
    pub const BRIGHTNESS_DOWN: u16 = 0x0070;
    pub const PLAY_PAUSE: u16 = 0x00CD;
    pub const NEXT_TRACK: u16 = 0x00B5;
    pub const PREV_TRACK: u16 = 0x00B6;
}

/// アプリからの命令（ring.ctrl に書かれたバイト列を解釈したもの）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Hello,
    Close,
    Open,
    Key { modifiers: u8, key: u8 },
    Consumer(u16),
    SetRotation(Rotation),
    GetSettings,
    SetScrollInvert(bool),
    SwitchProfile(u8),
}

impl Command {
    pub fn parse(d: &[u8]) -> Option<Command> {
        match *d {
            [ctrl::HELLO, ..] => Some(Command::Hello),
            [ctrl::CLOSE, ..] => Some(Command::Close),
            [ctrl::OPEN, ..] => Some(Command::Open),
            [ctrl::KEY, modifiers, key, ..] => Some(Command::Key { modifiers, key }),
            [ctrl::CONSUMER, lo, hi, ..] => Some(Command::Consumer(u16::from_le_bytes([lo, hi]))),
            [ctrl::SET_ROTATION, q, ..] => Rotation::from_quarters(q).map(Command::SetRotation),
            [ctrl::GET_SETTINGS, ..] => Some(Command::GetSettings),
            [ctrl::SET_SCROLL_INVERT, v, ..] => Some(Command::SetScrollInvert(v != 0)),
            [ctrl::SWITCH_PROFILE, p, ..] if (p as usize) < PROFILES => Some(Command::SwitchProfile(p)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Geometry =
        Geometry { cx: 1000, cy: 1000, rx: 500, ry: 500, swap_xy: false, invert_x: false, invert_y: false, rotation: Rotation::R0 };

    #[test]
    fn angles_are_clockwise_from_up() {
        // 補正なしなら、パッドの座標 = 画面の座標（x 右、y 下）
        assert_eq!(polar(&PLAIN, 1000, 500).0 / 100, 0); // 上
        assert_eq!(polar(&PLAIN, 1500, 1000).0 / 100, 90); // 右
        assert_eq!(polar(&PLAIN, 1000, 1500).0 / 100, 180); // 下
        assert_eq!(polar(&PLAIN, 500, 1000).0 / 100, 270); // 左
        let (a, _) = polar(&PLAIN, 1500, 500); // 右上
        assert!((4490..=4510).contains(&a), "{a}");
    }

    #[test]
    fn radius_is_normalized() {
        assert_eq!(polar(&PLAIN, 1000, 1000).1, 0);
        assert_eq!(polar(&PLAIN, 1500, 1000).1, 1000);
        assert_eq!(polar(&PLAIN, 1250, 1000).1, 500);
        let r = polar(&PLAIN, 1354, 1354).1; // 45° 方向に半径 ≒ 1000
        assert!((995..=1005).contains(&r), "{r}");
    }

    #[test]
    fn atan2_is_accurate() {
        for i in 0..360 {
            let t = (i as f32).to_radians();
            let (y, x) = (t.sin() * 700.0, t.cos() * 700.0);
            let want = y.atan2(x).to_degrees();
            let got = atan2_deg(y, x);
            let d = (got - want + 540.0).rem_euclid(360.0) - 180.0;
            assert!(d.abs() < 0.1, "{i}: {got} vs {want}");
        }
    }

    #[test]
    fn orientation_matches_the_pad_on_the_board() {
        // 実機（2026-10-21）: 画面の x = パッドの y、画面の y = -(パッドの x)
        let g = Geometry { cx: 1000, cy: 1000, rx: 500, ry: 500, ..Geometry::DEFAULT };
        // パッドの x が小さい側 → 画面の y が + → 下（180°）
        assert_eq!(polar(&g, 500, 1000).0 / 100, 180);
        // パッドの y が大きい側 → 画面の x が + → 右（90°）
        assert_eq!(polar(&g, 1000, 1500).0 / 100, 90);
    }

    #[test]
    fn rotation_adds_to_the_angle() {
        // 左に 90° 倒して置くと、本体の右（90°）が画面の上（0°）になる
        let g = Geometry { rotation: Rotation::R270, ..PLAIN };
        assert_eq!(polar(&g, 1500, 1000).0 / 100, 0);
        assert_eq!(polar(&g, 1000, 500).0 / 100, 270); // 本体の上 → 画面の左
        let g = Geometry { rotation: Rotation::R90, ..PLAIN };
        assert_eq!(polar(&g, 1000, 500).0 / 100, 90); // 本体の上 → 画面の右
    }

    #[test]
    fn parses_commands() {
        assert_eq!(Command::parse(&[0]), Some(Command::Hello));
        assert_eq!(Command::parse(&[1]), Some(Command::Close));
        assert_eq!(Command::parse(&[2]), Some(Command::Open));
        // Command + Shift + 4（スクリーンショット）
        assert_eq!(Command::parse(&[0x10, 0x0A, 0x21]), Some(Command::Key { modifiers: 0x0A, key: 0x21 }));
        assert_eq!(Command::parse(&[0x11, 0xE9, 0x00]), Some(Command::Consumer(consumer::VOLUME_UP)));
        assert_eq!(Command::parse(&[0x10, 0x08]), None, "too short");
        assert_eq!(Command::parse(&[]), None);
        assert_eq!(Command::parse(&[0x7F]), None);
        assert_eq!(Command::parse(&[0x20, 3]), Some(Command::SetRotation(Rotation::R270)));
        assert_eq!(Command::parse(&[0x20, 4]), None, "only 0..3");
        assert_eq!(Command::parse(&[0x21]), Some(Command::GetSettings));
        assert_eq!(Command::parse(&[0x22, 1]), Some(Command::SetScrollInvert(true)));
        assert_eq!(Command::parse(&[0x23, 1]), Some(Command::SwitchProfile(1)));
        assert_eq!(Command::parse(&[0x23, 2]), None, "only 2 profiles");
    }

    #[test]
    fn encodes_events() {
        let e = Event::Position { touching: true, angle_cdeg: 9000, radius: 800, x: 0x123, y: 0x456, z: 12 };
        assert_eq!(e.encode(7), [3, 1, 0x28, 0x23, 0x20, 0x03, 0x23, 0x01, 0x56, 0x04, 12, 7]);
        assert_eq!(Event::Opened.encode(0)[0], 1);
        assert_eq!(Event::Closed.encode(0)[0], 2);
        let e = Event::Settings { rotation: Rotation::R90, profile: 1, scroll_invert: true };
        assert_eq!(e.encode(5), [4, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 5]);
    }
}
