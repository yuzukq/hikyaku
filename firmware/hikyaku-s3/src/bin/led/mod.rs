//! オンボードの RGB LED（ESP32-S3 SuperMini の GPIO48。WS2812 互換）で状態を示す。
//!
//! RMT で WS2812 の信号を作る（80MHz・分周 1 = 12.5ns 刻み）。色は GRB の順に 24 ビット。
//! どのタスクからでも [`set`] で色を変えられる（チャネルはクリティカルセクションで守る）。

use core::cell::RefCell;

use critical_section::Mutex;
use esp_hal::{
    gpio::Level,
    peripherals::{GPIO48, RMT},
    rmt::{Channel, PulseCode, Rmt, Tx, TxChannelConfig, TxChannelCreator},
    time::Rate,
    Blocking,
};

/// 明るさを抑えた色（眩しくないように、各色 0〜255 のうち小さめの値にする）。
pub const RED: (u8, u8, u8) = (24, 0, 0);
pub const GREEN: (u8, u8, u8) = (0, 24, 0);
pub const BLUE: (u8, u8, u8) = (0, 0, 32);
/// USB キーボードの状態（診断用）。
pub const WHITE: (u8, u8, u8) = (16, 16, 16);
pub const CYAN: (u8, u8, u8) = (0, 20, 20);
pub const YELLOW: (u8, u8, u8) = (20, 16, 0);
pub const PURPLE: (u8, u8, u8) = (16, 0, 24);
pub const ORANGE: (u8, u8, u8) = (24, 8, 0);

// WS2812 のビットの長さ（12.5ns 単位）。0: High 0.4µs / Low 0.85µs、1: High 0.8µs / Low 0.45µs。
const T0H: u16 = 32;
const T0L: u16 = 68;
const T1H: u16 = 64;
const T1L: u16 = 36;

static LED: Mutex<RefCell<Option<Channel<'static, Blocking, Tx>>>> = Mutex::new(RefCell::new(None));
/// 今の色（一瞬だけ光らせた後に戻すため）。
static CURRENT: Mutex<RefCell<(u8, u8, u8)>> = Mutex::new(RefCell::new((0, 0, 0)));

/// 今の色。
pub fn current() -> (u8, u8, u8) {
    critical_section::with(|cs| *CURRENT.borrow(cs).borrow())
}

/// `color` で `ms` ミリ秒だけ光らせて、元の色に戻す。
pub async fn flash(color: (u8, u8, u8), ms: u64) {
    let prev = current();
    set(color);
    embassy_time::Timer::after(embassy_time::Duration::from_millis(ms)).await;
    set(prev);
}

/// RMT のチャネル 0 を GPIO48 に割り当てる。失敗しても何もしない（LED が点かないだけ）。
pub fn init(rmt: RMT<'static>, pin: GPIO48<'static>) {
    let Ok(rmt) = Rmt::new(rmt, Rate::from_mhz(80)) else {
        return;
    };
    let cfg = TxChannelConfig::default()
        .with_clk_divider(1)
        .with_idle_output_level(Level::Low)
        .with_idle_output(true)
        .with_carrier_modulation(false);
    let Ok(ch) = rmt.channel0.configure_tx(&cfg) else {
        return;
    };
    let ch = ch.with_pin(pin);
    critical_section::with(|cs| LED.borrow(cs).replace(Some(ch)));
}

/// LED の色を変える（`(r, g, b)`）。
pub fn set((r, g, b): (u8, u8, u8)) {
    critical_section::with(|cs| *CURRENT.borrow(cs).borrow_mut() = (r, g, b));
    let mut data = [PulseCode::end_marker(); 25];
    let grb = (g as u32) << 16 | (r as u32) << 8 | b as u32;
    for (i, code) in data.iter_mut().take(24).enumerate() {
        *code = if grb & (1 << (23 - i)) != 0 {
            PulseCode::new(Level::High, T1H, Level::Low, T1L)
        } else {
            PulseCode::new(Level::High, T0H, Level::Low, T0L)
        };
    }
    critical_section::with(|cs| {
        let mut slot = LED.borrow(cs).borrow_mut();
        let Some(ch) = slot.take() else {
            return;
        };
        let ch = match ch.transmit(&data) {
            Ok(tx) => match tx.wait() {
                Ok(ch) | Err((_, ch)) => ch,
            },
            Err((_, ch)) => ch,
        };
        slot.replace(ch);
    });
}
