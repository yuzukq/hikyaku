//! トラックパッド（Cirque Pinnacle, SPI）→ マウスレポート。`--features trackpad` のときだけ有効。
//!
//! 配線（docs/hardware/wiring.md）: SCK=GPIO12 / MISO=GPIO13 / MOSI=GPIO11 / SS=GPIO10 / DR=GPIO9（SPI2 の標準ピン + DR）。
//! SPI モード 1、250kHz（1MHz では読み取りエラーが多かった）。疎通は実機で確認済み（2026-10-11）。

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_time::{Duration, Instant, Timer};
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::{
    gpio::{Input, Output},
    spi::master::Spi,
    Blocking,
};
use hikyaku_pinnacle::Pinnacle;
use hikyaku_pointer::{Config, Pointer};
use log::{info, warn};

use crate::{Report, REPORTS};

/// true の間、トラックパッドの動きをスクロールに変換する（キー押下中など。今は常に false）。
pub static SCROLL_MODE: AtomicBool = AtomicBool::new(false);

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
    match pad.init() {
        // 参考実装では ID=0x07, Version=0x3A。違っていても動くことはあるので警告に留める。
        Ok(i) => info!("trackpad: firmware id={:#04x} version={:#04x}", i.firmware_id, i.firmware_version),
        Err(_) => {
            warn!("trackpad: init failed (check wiring)");
            return;
        }
    }

    let mut pointer = Pointer::new(Config::default());
    let mut touch: Option<(u16, u16)> = None;

    // 診断用の集計（1 秒ごとにログへ）。実機でカーソルが動かないときの切り分け用。
    let mut polls = 0u32;
    let mut packets = 0u32;
    let mut touch_packets = 0u32;
    let mut dr_high = 0u32;
    let mut reports = 0u32;
    let mut read_errors = 0u32;
    let mut disagreed = 0u32;
    let mut last_log = Instant::now();
    let mut last_raw = (0u16, 0u16, 0u8);
    let mut was_touching = false;

    loop {
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
                packets += 1;
                last_raw = (d.x, d.y, d.z);
                #[cfg(feature = "trackpad-trace")]
                info!("tp {} {} {} {}", Instant::now().as_millis() as u32, d.x, d.y, d.z);
                if d.touching() {
                    touch_packets += 1;
                }
                touch = d.touching().then_some((d.x, d.y));
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
                    "trackpad: polls={} packets={} touching={} dr_high={} reports={} errors={} disagreed={} status1={:?} last(x,y,z)={:?}",
                    polls, packets, touch_packets, dr_high, reports, read_errors, disagreed, status1, last_raw
                );
            }
            polls = 0;
            packets = 0;
            touch_packets = 0;
            dr_high = 0;
            reports = 0;
            read_errors = 0;
            disagreed = 0;
            last_log = Instant::now();
        }
        // Pinnacle のサンプルレートは約 100Hz。それに合わせる。
        Timer::after(Duration::from_millis(10)).await;
    }
}
