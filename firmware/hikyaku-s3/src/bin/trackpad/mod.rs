//! トラックパッド（Cirque Pinnacle, SPI）→ マウスレポート。`--features trackpad` のときだけ有効。
//!
//! **相対モード**（パッド自身が移動量とタップを計算する。QMK・ZMK でも実績のある方式）。
//! 絶対座標から移動量を自前で作る旧方式は 2026-10-13 に削除した（タグ `absolute-mode-last` に残っている）。
//!
//! 配線（docs/hardware/wiring.md）: SCK=GPIO12 / MISO=GPIO13 / MOSI=GPIO11 / SS=GPIO10 / DR=GPIO9（SPI2 の標準ピン + DR）。
//! SPI モード 1、**2MHz**。1MHz 以下は MSB を取り違える。4MHz は読み戻しテストは通ったが、実機で相対モードの Y が出ず
//! 動きも途切れがちだったので、実績のある 2MHz にしている。このパッドは SPI のクロックが低い（1MHz 以下）と MSB を取り違える。
//! レジスタの読み戻しテストで、2MHz・4MHz は 0 エラー、250k〜1MHz は約半分が誤りと確認した（2026-10-13）。

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_time::{Duration, Instant, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::{
    delay::Delay,
    gpio::{Input, Output},
    spi::master::Spi,
    Blocking,
};
#[cfg(feature = "trackpad-selftest")]
use esp_hal::{
    spi::{master::Config as SpiConfig, Mode},
    time::Rate,
};
use hikyaku_pinnacle::{Attenuation, Pinnacle, RelativeOptions};
use hikyaku_pointer::relative::{Config as RelayConfig, Relay, SoftTap, StillFilter};
use log::{info, warn};

use crate::{Report, REPORTS};

/// true の間、トラックパッドの動きをスクロールに変換する（キー押下中など。今は常に false）。
pub static SCROLL_MODE: AtomicBool = AtomicBool::new(false);

/// SPI の読み書きの総当たりテスト。パッドを触らなくても、レジスタに既知のパターンを書いて読み戻すだけで、
/// 「SPI の読み取りが正しいか」を判定できる（MSB が落ちる、ビットがずれる、など）。
/// 使うレジスタは ZIdle（0x0A）と ERA の値・アドレス（0x1B〜0x1D。CTRL に書くまで何も起きない）。終わったら元に戻す。
#[cfg(feature = "trackpad-selftest")]
fn selftest(pad: &mut Pinnacle<ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, Delay>>) {
    use hikyaku_pinnacle::reg;
    const PATTERNS: [u8; 10] = [0x00, 0xFF, 0x80, 0x7F, 0xAA, 0x55, 0x01, 0xFE, 0xC3, 0x3C];
    let saved_z = pad.read_reg(reg::Z_IDLE).unwrap_or(0x1E);
    info!("selftest: start (ZIdle was {:#04x})", saved_z);
    for (mode, mname) in [(Mode::_1, 1u8)] {
        for khz in [1000u32, 2000, 4000] {
            for setup_ns in [0u32, 10_000] {
                let cfg = SpiConfig::default().with_frequency(Rate::from_khz(khz)).with_mode(mode);
                if pad.spi_mut().bus_mut().apply_config(&cfg).is_err() {
                    continue;
                }
                pad.set_cs_setup_ns(setup_ns);
                let (mut total, mut single_bad, mut burst_bad) = (0u32, 0u32, 0u32);
                let (mut first_single, mut first_burst) = (None, None);
                for &pat in &PATTERNS {
                    for _ in 0..30 {
                        total += 1;
                        // 1 バイトの書き込み → 読み戻し
                        let _ = pad.write_reg(reg::Z_IDLE, pat);
                        let got = pad.read_reg(reg::Z_IDLE).unwrap_or(0xEE);
                        if got != pat {
                            single_bad += 1;
                            first_single.get_or_insert((pat, got));
                        }
                        // 3 バイトの連続書き込み → 連続読み出し（自動インクリメントとバイト位置の確認）
                        let want = [pat, !pat, pat ^ 0x5A];
                        for (i, w) in want.iter().enumerate() {
                            let _ = pad.write_reg(reg::ERA_VALUE + i as u8, *w);
                        }
                        let mut got3 = [0u8; 3];
                        let _ = pad.read_regs(reg::ERA_VALUE, &mut got3);
                        if got3 != want {
                            burst_bad += 1;
                            first_burst.get_or_insert((want, got3));
                        }
                    }
                }
                info!(
                    "selftest mode={} khz={} cs_setup={}ns: single {}/{} bad, burst {}/{} bad, first_single(want,got)={:?} first_burst={:?}",
                    mname, khz, setup_ns, single_bad, total, burst_bad, total, first_single, first_burst
                );
            }
        }
    }
    // 元に戻す
    let cfg = SpiConfig::default().with_frequency(Rate::from_khz(2000)).with_mode(Mode::_1);
    let _ = pad.spi_mut().bus_mut().apply_config(&cfg);
    pad.set_cs_setup_ns(0);
    let _ = pad.write_reg(reg::Z_IDLE, saved_z);
    for i in 0..3u8 {
        let _ = pad.write_reg(reg::ERA_VALUE + i, 0);
    }
    info!("selftest: done");
}

/// タッチ感度（ADC 減衰）。既定は **2X**（2026-10-15、基板化後の実機で決めた）。4X（いちばん鈍い。QMK の既定）では指を検出したり
/// しなかったりして、動きが途切れ、左右がほとんど出なかった。2X で動き・タップとも良好。
/// `trackpad-sens-1x` / `-3x` / `-4x` で変える（1X がいちばん敏感）。`trackpad-sens-2x` は既定と同じ。
/// 変えるとキャリブレーションが走る（起動中はパッドに触れないこと）。
#[cfg(any(
    all(feature = "trackpad-sens-1x", feature = "trackpad-sens-3x"),
    all(feature = "trackpad-sens-1x", feature = "trackpad-sens-4x"),
    all(feature = "trackpad-sens-3x", feature = "trackpad-sens-4x"),
))]
compile_error!("trackpad-sens-1x / -3x / -4x のうち 1 つだけを指定してください");
const SENSITIVITY: Attenuation = if cfg!(feature = "trackpad-sens-1x") {
    Attenuation::X1
} else if cfg!(feature = "trackpad-sens-3x") {
    Attenuation::X3
} else if cfg!(feature = "trackpad-sens-4x") {
    Attenuation::X4
} else {
    Attenuation::X2
};

/// 相対モードの設定（タップ有効、感度 2X、ZIdle 0）。
/// **パッド側の反転（invert_x/invert_y）は使わない。** データシートでは絶対座標モード用で、実機（2026-10-13）では
/// Y 反転を入れると相対モードの Y の移動量がほとんど出なくなった。向きはソフト側（`SWAP_XY`・`SCREEN_*_INVERT`）で合わせる。
///
/// `trackpad-soft-tap` のときはパッド側のタップを切り、指を離した合図（ゼロパケット）のために ZIdle = 5 にする。
const REL_OPTIONS: RelativeOptions = RelativeOptions {
    sensitivity: SENSITIVITY,
    primary_tap: !cfg!(feature = "trackpad-soft-tap"),
    invert_x: false,
    invert_y: false,
    swap_xy: false,
    z_idle: if cfg!(feature = "trackpad-soft-tap") { 5 } else { 0 },
};

/// パッドの軸と画面の軸の対応（基板裏の印字が読める向きを「上」とする）。
/// 実機（2026-10-13）: 上→下のスワイプでパッドの dx が負になっていた（パッドが 90 度回った向きで付いている）。
/// そこで X と Y を入れ替え、画面の Y はパッドの dx の符号を反転したものにする。
/// 左右が逆なら `SCREEN_X_INVERT` を、上下が逆なら `SCREEN_Y_INVERT` を切り替える。
const SWAP_XY: bool = true;
const SCREEN_X_INVERT: bool = true;
const SCREEN_Y_INVERT: bool = true;

/// 移動量の倍率。パッドの相対モードの単位は粗く（31mm のパッドを端から端までなぞって 150〜200 程度）、1:1 では画面上の移動が
/// 小さすぎる。QMK の相対モードの既定は 4 倍（`scale 1024 / 256`）だが、実機で速すぎたので 2 倍（2026-10-13）。
const REL_GAIN: i32 = 2;

/// スクロールモードで、ホイール 1 目盛りに相当する移動量。
const SCROLL_DIV: i32 = 8;

/// 相対モードの変換の設定（上の定数をまとめたもの。`hikyaku_pointer::relative::Config::FIRMWARE_DEFAULT` と同じ値）。
fn relay_config() -> RelayConfig {
    RelayConfig {
        swap_xy: SWAP_XY,
        invert_x: SCREEN_X_INVERT,
        invert_y: SCREEN_Y_INVERT,
        gain: REL_GAIN,
        scroll_div: SCROLL_DIV,
        // 静止中のぶれを抑える（`trackpad-still-filter`）。既定は無効。
        still: cfg!(feature = "trackpad-still-filter").then_some(StillFilter::DEFAULT),
        // 自前のタップ判定（`trackpad-soft-tap`）。既定はパッド側のタップ。
        soft_tap: cfg!(feature = "trackpad-soft-tap").then_some(SoftTap::DEFAULT),
    }
}

/// 相対モード（既定）。ZMK（Zephyr の input_pinnacle）や Cirque 公式の USB マウスのサンプルと同じく、
/// **パッドが計算した移動量をそのまま**（向きを合わせ、`REL_GAIN` 倍して）マウスレポートにする（加速は OS に任せる）。
#[embassy_executor::task]
pub async fn run(spi: Spi<'static, Blocking>, cs: Output<'static>, _dr: Input<'static>) {
    let dev = match ExclusiveDevice::new(spi, cs, Delay::new()) {
        Ok(d) => d,
        Err(_) => {
            warn!("trackpad: chip select setup failed");
            return;
        }
    };
    let mut pad = Pinnacle::new(dev);

    #[cfg(feature = "trackpad-selftest")]
    selftest(&mut pad);

    // 起動時はキャリブレーションが走る。パッドに触れないこと（補正が狂う）。
    match pad.init_relative(&mut Delay::new(), REL_OPTIONS) {
        Ok(i) => {
            info!("trackpad: relative mode, firmware id={:#04x} version={:#04x}", i.firmware_id, i.firmware_version);
            crate::led::set(crate::led::GREEN); // キャリブレーション完了。触ってよい
        }
        Err(hikyaku_pinnacle::Error::BadId(id)) => {
            crate::led::set(crate::led::BLUE);
            // 配線（MISO・電源・GND・CS）が外れていると、0x00 や 0xFC が読める。ゴミを送らないよう、ここで止める。
            warn!("trackpad: wrong firmware id {:#04x} (expected 0x07). Check the 12-pin FFC / wiring. Trackpad disabled.", id);
            return;
        }
        Err(_) => {
            crate::led::set(crate::led::BLUE);
            warn!("trackpad: init failed (check wiring)");
            return;
        }
    }

    // 変換（軸の入れ替え・反転・倍率・持ち越し・レポートの分割）は `hikyaku_pointer::relative` にある。
    // ホストで実機ログを再生してテストできる（`cargo test -p hikyaku-pointer`、`examples/replay_rel.rs`）。
    let mut relay = Relay::new(relay_config());

    // 診断用の集計（1 秒ごと）
    let (mut packets, mut reports, mut errors) = (0u32, 0u32, 0u32);
    let (mut sum_dx, mut sum_dy, mut max_step, mut clicks) = (0i32, 0i32, 0i32, 0u32);
    let mut last_buttons = 0u8;
    // 送れずに捨てたレポート数（BLE 側のキューがいっぱい）と、読み取りの間隔の最大（タスクが止まっていないか）
    let (mut dropped, mut max_gap_ms) = (0u32, 0u64);
    let mut last_poll = Instant::now();
    let mut last_log = Instant::now();

    loop {
        max_gap_ms = max_gap_ms.max(last_poll.elapsed().as_millis());
        last_poll = Instant::now();
        let now = Instant::now().as_millis() as u32;
        // 出すレポートを BLE へ流し、集計する
        let mut emit = |r: [u8; 5]| {
            if r[0] & 1 != 0 && last_buttons & 1 == 0 {
                clicks += 1;
            }
            last_buttons = r[0];
            if REPORTS.try_send(Report::Mouse(r)).is_err() {
                dropped += 1;
            }
            reports += 1;
        };
        match pad.read_relative() {
            Ok(Some(r)) => {
                packets += 1;
                sum_dx += r.dx as i32;
                sum_dy += r.dy as i32;
                max_step = max_step.max((r.dx as i32).abs()).max((r.dy as i32).abs());
                #[cfg(feature = "trackpad-trace")]
                info!("rel {} raw={:02x?} dx={} dy={}", now, r.raw, r.dx, r.dy);
                relay.push(now, &r, SCROLL_MODE.load(Ordering::Relaxed), &mut emit);
            }
            Ok(None) => relay.poll(now, &mut emit),
            Err(_) => errors += 1,
        }
        if last_log.elapsed() >= Duration::from_secs(1) {
            if packets > 0 || errors > 0 {
                info!(
                    "trackpad: packets={} reports={} dropped={} max_gap_ms={} sum(dx,dy)=({},{}) max|step|={} clicks={} errors={}",
                    packets, reports, dropped, max_gap_ms, sum_dx, sum_dy, max_step, clicks, errors
                );
            }
            (packets, reports, errors, sum_dx, sum_dy, max_step, clicks) = (0, 0, 0, 0, 0, 0, 0);
            (dropped, max_gap_ms) = (0, 0);
            last_log = Instant::now();
        }
        // パッドは約 100Hz で出す。取りこぼさないよう、その倍の頻度で見る。
        Timer::after(Duration::from_millis(5)).await;
    }
}
