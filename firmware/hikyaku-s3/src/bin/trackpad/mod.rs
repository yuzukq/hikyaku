//! トラックパッド（Cirque Pinnacle, SPI）→ マウスレポート。`--features trackpad` のときだけ有効。
//!
//! 既定は**相対モード**（パッド自身が移動量とタップを計算する。QMK でも実績のある方式）。
//! `trackpad-absolute` を付けると、絶対座標から移動量を自前で作る旧方式になる。
//!
//! 配線（docs/hardware/wiring.md）: SCK=GPIO12 / MISO=GPIO13 / MOSI=GPIO11 / SS=GPIO10 / DR=GPIO9（SPI2 の標準ピン + DR）。
//! SPI モード 1、250kHz（1MHz では読み取りエラーが多かった）。疎通は実機で確認済み（2026-10-11）。

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_time::{Duration, Instant, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::{
    delay::Delay,
    gpio::{Input, Output},
    spi::master::Spi,
    Blocking,
};
#[cfg(feature = "trackpad-sweep")]
use esp_hal::{
    spi::{master::Config as SpiConfig, Mode},
    time::Rate,
};
use hikyaku_pinnacle::{Attenuation, Pinnacle};
#[cfg(feature = "trackpad-lowbyte")]
use hikyaku_pointer::tracker::LowByteTracker;
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

/// 相対モード用のゲイン（1/256 単位）。パッドの移動量の単位は細かくないので、QMK の既定（4 倍）に近い値から始める。
/// 速すぎる・遅すぎるときはここを調整する。
#[cfg(not(feature = "trackpad-absolute"))]
const REL_GAIN_Q8: u32 = 512;

#[cfg(not(feature = "trackpad-absolute"))]
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
    // タップはパッドに任せる（Relative::buttons に出る）。ADC 減衰は QMK の既定（4X）。
    match pad.init_relative(&mut Delay::new(), Attenuation::X4, true) {
        Ok(i) => info!("trackpad: relative mode, firmware id={:#04x} version={:#04x}", i.firmware_id, i.firmware_version),
        Err(_) => {
            warn!("trackpad: init failed (check wiring)");
            return;
        }
    }

    let cfg = Config { gain_q8: REL_GAIN_Q8, accel_ref: 0, ..Config::default() };
    let mut pointer = Pointer::new(cfg);

    // 診断用の集計（1 秒ごと）
    let (mut packets, mut reports, mut errors) = (0u32, 0u32, 0u32);
    let (mut sum_dx, mut sum_dy, mut max_step, mut clicks) = (0u32, 0u32, 0i32, 0u32);
    let mut last_buttons = 0u8;
    let mut last_log = Instant::now();

    loop {
        match pad.read_relative() {
            Ok(Some(r)) => {
                packets += 1;
                sum_dx += r.dx.unsigned_abs() as u32;
                sum_dy += r.dy.unsigned_abs() as u32;
                max_step = max_step.max((r.dx as i32).abs()).max((r.dy as i32).abs());
                if r.buttons & 1 != 0 && last_buttons & 1 == 0 {
                    clicks += 1;
                }
                last_buttons = r.buttons;
                #[cfg(feature = "trackpad-trace")]
                info!("rel {} dx={} dy={} btn={} wheel={}", Instant::now().as_millis() as u32, r.dx, r.dy, r.buttons, r.wheel);
                let now = Instant::now().as_millis() as u32;
                if let Some(rep) =
                    pointer.update_relative(now, r.dx as i32, r.dy as i32, r.buttons, SCROLL_MODE.load(Ordering::Relaxed))
                {
                    reports += 1;
                    let _ = REPORTS.try_send(Report::Mouse(rep.to_bytes()));
                }
            }
            Ok(None) => {}
            Err(_) => errors += 1,
        }
        if last_log.elapsed() >= Duration::from_secs(1) {
            if packets > 0 || errors > 0 {
                info!(
                    "trackpad: packets={} reports={} sum|dx|={} sum|dy|={} max|step|={} clicks={} errors={}",
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
