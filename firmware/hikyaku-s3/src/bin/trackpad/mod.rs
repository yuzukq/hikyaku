//! トラックパッド（Cirque Pinnacle, SPI）→ マウスレポート。`--features trackpad` のときだけ有効。
//!
//! 配線（docs/hardware/wiring.md）: SCK=GPIO12 / MISO=GPIO13 / MOSI=GPIO11 / SS=GPIO10 / DR=GPIO9（SPI2 の標準ピン + DR）。
//! SPI モード 1、1MHz。実機は未確認（コネクタ待ち）。

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
    loop {
        if dr.is_high() {
            match pad.read_absolute() {
                Ok(Some(d)) => touch = d.touching().then_some((d.x, d.y)),
                Ok(None) => {}
                Err(_) => warn!("trackpad: read failed"),
            }
        }
        let now = Instant::now().as_millis() as u32;
        if let Some(r) = pointer.update(now, touch, SCROLL_MODE.load(Ordering::Relaxed)) {
            let _ = REPORTS.try_send(Report::Mouse(r.to_bytes()));
        }
        Timer::after(Duration::from_millis(5)).await;
    }
}
