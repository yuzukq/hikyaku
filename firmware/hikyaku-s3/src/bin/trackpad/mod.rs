//! トラックパッド（Cirque Pinnacle, SPI）→ マウスレポート。`--features trackpad` のときだけ有効。
//!
//! 既定は**相対モード**（パッド自身が移動量とタップを計算する。QMK でも実績のある方式）。
//! `trackpad-absolute` を付けると、絶対座標から移動量を自前で作る旧方式になる。
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
#[cfg(any(feature = "trackpad-sweep", feature = "trackpad-selftest"))]
use esp_hal::{
    spi::{master::Config as SpiConfig, Mode},
    time::Rate,
};
use hikyaku_pinnacle::{Attenuation, Pinnacle, RelativeOptions};
#[cfg(feature = "trackpad-lowbyte")]
use hikyaku_pointer::tracker::LowByteTracker;
#[cfg(not(feature = "trackpad-absolute"))]
use hikyaku_pointer::relative::{Config as RelayConfig, Relay, SoftTap, StillFilter};
#[cfg(feature = "trackpad-absolute")]
use hikyaku_pointer::{Config, Pointer};
use log::{info, warn};

use crate::{Report, REPORTS};

/// 調査用（`trackpad-trace`）: SPI の周波数を一定時間ごとに切り替えて、MSB が正しく読めているかを周波数ごとに調べる。
#[cfg(feature = "trackpad-sweep")]
const SWEEP_KHZ: [u32; 6] = [4000, 2000, 1000, 500, 250, 8000];
#[cfg(feature = "trackpad-sweep")]
const SWEEP_SECS: u64 = 8;

/// true の間、トラックパッドの動きをスクロールに変換する（キー押下中など。今は常に false）。
pub static SCROLL_MODE: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "trackpad-absolute")]
#[embassy_executor::task]
pub async fn run(spi: Spi<'static, Blocking>, cs: Output<'static>, dr: Input<'static>) {
    let dev = match ExclusiveDevice::new_no_delay(spi, cs) {
        Ok(d) => d,
        Err(_) => {
            warn!("trackpad: chip select setup failed");
            return;
        }
    };
    let mut pad = Pinnacle::new(dev);
    // 起動時はキャリブレーションが走る。パッドに触れないこと（補正が狂う）。
    // ADC 減衰は QMK の既定（平らなオーバーレイで 4X）。オーバーレイの厚みに合わせて調整する。
    match pad.init(&mut Delay::new(), Attenuation::X4) {
        // 参考実装では ID=0x07, Version=0x3A。違っていても動くことはあるので警告に留める。
        Ok(i) => info!("trackpad: firmware id={:#04x} version={:#04x}", i.firmware_id, i.firmware_version),
        Err(_) => {
            warn!("trackpad: init failed (check wiring)");
            return;
        }
    }

    // 既定は QMK と同じく、報告された絶対座標をそのまま使う。
    // `trackpad-lowbyte` のときは、下位バイトの前後差から位置を作る（tracker.rs。追いかけた位置は u16 の中央付近なので、
    // クリップ範囲を外す）。
    #[allow(unused_mut)]
    let mut cfg = Config::default();
    #[cfg(feature = "trackpad-lowbyte")]
    {
        cfg.x_min = 0;
        cfg.x_max = u16::MAX;
        cfg.y_min = 0;
        cfg.y_max = u16::MAX;
    }
    let mut pointer = Pointer::new(cfg);
    #[cfg(feature = "trackpad-lowbyte")]
    let mut tracker = LowByteTracker::new();
    let mut touch: Option<(u16, u16)> = None;

    // 診断用の集計（1 秒ごとにログへ）。実機でカーソルが動かないときの切り分け用。
    let mut polls = 0u32;
    let mut packets = 0u32;
    let mut touch_packets = 0u32;
    let mut dr_high = 0u32;
    let mut reports = 0u32;
    let mut read_errors = 0u32;
    let mut disagreed = 0u32;
    // 下位バイトの bit7（128 の位）が立っていたサンプル数。正しく読めていれば、触っている間はおよそ半分のサンプルで立つ。
    let mut torn = 0u32;
    let mut prev_pkt: Option<([u8; 6], u16, u16)> = None;
    let mut msb_x = 0u32;
    let mut msb_y = 0u32;
    let mut khz = 0u32;
    #[cfg(feature = "trackpad-sweep")]
    let mut sweep_idx = 0usize;
    // 起動直後は経過時間が短く、`Instant::now() - Duration` はアンダーフローしてパニックする。引き算はしない。
    #[cfg(feature = "trackpad-sweep")]
    let mut last_switch = Instant::now();
    #[cfg(feature = "trackpad-sweep")]
    let mut first_switch = true;
    let mut last_log = Instant::now();
    let mut last_raw = (0u16, 0u16, 0u8);
    let mut was_touching = false;

    loop {
        #[cfg(feature = "trackpad-sweep")]
        if first_switch || last_switch.elapsed() >= Duration::from_secs(SWEEP_SECS) {
            first_switch = false;
            khz = SWEEP_KHZ[sweep_idx % SWEEP_KHZ.len()];
            sweep_idx += 1;
            last_switch = Instant::now();
            let cfg = SpiConfig::default().with_frequency(Rate::from_khz(khz)).with_mode(Mode::_1);
            if pad.spi_mut().bus_mut().apply_config(&cfg).is_err() {
                warn!("trackpad: could not set SPI to {} kHz", khz);
            }
            info!("trackpad: SPI now {} kHz", khz);
        }
        // DR ピンに頼らず Status1 を毎回見る（DR の配線が不安定でも読めるように）。
        polls += 1;
        if dr.is_high() {
            dr_high += 1;
        }
        match pad.read_absolute_checked() {
            Ok(Some(c)) => {
                let d = c.data;
                if c.disagreed {
                    disagreed += 1;
                }
                // 座標の上位ビットが下位バイトより遅れて更新される現象の検出。下位バイトの前後差（256 を法とする）と、
                // 報告された座標の差が大きくずれたら「ずれ」と数える。キャリブレーション後は 0 になるはず。
                if c.data.touching() {
                    if let Some((pr, px, py)) = prev_pkt {
                        let sd = |a: u8, b: u8| (b.wrapping_sub(a) as i8) as i32;
                        let (sdx, sdy) = (sd(pr[2], c.raw[2]), sd(pr[3], c.raw[3]));
                        let (rdx, rdy) = (d.x as i32 - px as i32, d.y as i32 - py as i32);
                        if (sdx.abs() < 60 && (rdx - sdx).abs() > 100) || (sdy.abs() < 60 && (rdy - sdy).abs() > 100) {
                            torn += 1;
                        }
                    }
                    prev_pkt = Some((c.raw, d.x, d.y));
                } else {
                    prev_pkt = None;
                }
                if c.data.touching() {
                    // 生パケットの X 下位バイト(2)・Y 下位バイト(3)
                    if c.raw[2] & 0x80 != 0 {
                        msb_x += 1;
                    }
                    if c.raw[3] & 0x80 != 0 {
                        msb_y += 1;
                    }
                }
                packets += 1;
                last_raw = (d.x, d.y, d.z);
                #[cfg(feature = "trackpad-trace")]
                info!("tp {} {} {} {} khz={} raw={:02x?}", Instant::now().as_millis() as u32, d.x, d.y, d.z, khz, c.raw);
                if d.touching() {
                    touch_packets += 1;
                }
                touch = if d.touching() {
                    #[cfg(feature = "trackpad-lowbyte")]
                    let pos = tracker.update(c.raw[2], c.raw[3]);
                    #[cfg(not(feature = "trackpad-lowbyte"))]
                    let pos = (d.x, d.y);
                    Some(pos)
                } else {
                    #[cfg(feature = "trackpad-lowbyte")]
                    tracker.reset();
                    None
                };
                if d.touching() != was_touching {
                    info!("trackpad: {} x={} y={} z={}", if d.touching() { "touch" } else { "release" }, d.x, d.y, d.z);
                    was_touching = d.touching();
                }
            }
            Ok(None) => {}
            Err(_) => read_errors += 1,
        }
        let now = Instant::now().as_millis() as u32;
        if let Some(r) = pointer.update(now, touch, SCROLL_MODE.load(Ordering::Relaxed)) {
            reports += 1;
            let _ = REPORTS.try_send(Report::Mouse(r.to_bytes()));
        }

        if last_log.elapsed() >= Duration::from_secs(1) {
            // 何も起きていない間は出さない（ログが流れすぎないように）
            if packets > 0 || reports > 0 || dr_high > 0 || read_errors > 0 {
                let status1 = pad.read_reg(hikyaku_pinnacle::reg::STATUS1).ok();
                info!(
                    "trackpad: khz={} packets={} touching={} torn={} reports={} errors={} disagreed={} status1={:?} last(x,y,z)={:?}",
                    khz, packets, touch_packets, torn, reports, read_errors, disagreed, status1, last_raw
                );
            }
            polls = 0;
            packets = 0;
            touch_packets = 0;
            dr_high = 0;
            reports = 0;
            read_errors = 0;
            disagreed = 0;
            msb_x = 0;
            msb_y = 0;
            torn = 0;
            last_log = Instant::now();
        }
        // Pinnacle のサンプルレートは約 100Hz。それに合わせる。
        Timer::after(Duration::from_millis(10)).await;
    }
}


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

/// 相対モードの設定（タップ有効、感度 4X、ZIdle 0）。
/// **パッド側の反転（invert_x/invert_y）は使わない。** データシートでは絶対座標モード用で、実機（2026-10-13）では
/// Y 反転を入れると相対モードの Y の移動量がほとんど出なくなった。向きはソフト側（`SWAP_XY`・`SCREEN_*_INVERT`）で合わせる。
///
/// `trackpad-soft-tap` のときはパッド側のタップを切り、指を離した合図（ゼロパケット）のために ZIdle = 5 にする。
#[cfg(not(feature = "trackpad-absolute"))]
const REL_OPTIONS: RelativeOptions = RelativeOptions {
    sensitivity: Attenuation::X4,
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
#[cfg(not(feature = "trackpad-absolute"))]
const SWAP_XY: bool = true;
#[cfg(not(feature = "trackpad-absolute"))]
const SCREEN_X_INVERT: bool = true;
#[cfg(not(feature = "trackpad-absolute"))]
const SCREEN_Y_INVERT: bool = true;

/// 移動量の倍率。パッドの相対モードの単位は粗く（31mm のパッドを端から端までなぞって 150〜200 程度）、1:1 では画面上の移動が
/// 小さすぎる。QMK の相対モードの既定は 4 倍（`scale 1024 / 256`）だが、実機で速すぎたので 2 倍（2026-10-13）。
#[cfg(not(feature = "trackpad-absolute"))]
const REL_GAIN: i32 = 2;

/// スクロールモードで、ホイール 1 目盛りに相当する移動量。
#[cfg(not(feature = "trackpad-absolute"))]
const SCROLL_DIV: i32 = 8;

/// 相対モードの変換の設定（上の定数をまとめたもの。`hikyaku_pointer::relative::Config::FIRMWARE_DEFAULT` と同じ値）。
#[cfg(not(feature = "trackpad-absolute"))]
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
#[cfg(not(feature = "trackpad-absolute"))]
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
        Ok(i) => info!("trackpad: relative mode, firmware id={:#04x} version={:#04x}", i.firmware_id, i.firmware_version),
        Err(hikyaku_pinnacle::Error::BadId(id)) => {
            // 配線（MISO・電源・GND・CS）が外れていると、0x00 や 0xFC が読める。ゴミを送らないよう、ここで止める。
            warn!("trackpad: wrong firmware id {:#04x} (expected 0x07). Check the 12-pin FFC / wiring. Trackpad disabled.", id);
            return;
        }
        Err(_) => {
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
    let mut last_log = Instant::now();

    loop {
        let now = Instant::now().as_millis() as u32;
        // 出すレポートを BLE へ流し、集計する
        let mut emit = |r: [u8; 5]| {
            if r[0] & 1 != 0 && last_buttons & 1 == 0 {
                clicks += 1;
            }
            last_buttons = r[0];
            let _ = REPORTS.try_send(Report::Mouse(r));
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
                    "trackpad: packets={} reports={} sum(dx,dy)=({},{}) max|step|={} clicks={} errors={}",
                    packets, reports, sum_dx, sum_dy, max_step, clicks, errors
                );
            }
            (packets, reports, errors, sum_dx, sum_dy, max_step, clicks) = (0, 0, 0, 0, 0, 0, 0);
            last_log = Instant::now();
        }
        // パッドは約 100Hz で出す。取りこぼさないよう、その倍の頻度で見る。
        Timer::after(Duration::from_millis(5)).await;
    }
}
