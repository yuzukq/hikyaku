#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use esp_hal::{
    clock::CpuClock,
    main,
    uart::{Config as UartConfig, Uart},
    usb::usb_serial_jtag::UsbSerialJtag,
};

use esp_backtrace as _;


// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    // generator version: 1.4.0
    // generator parameters: -o esp32c3 -o unstable-hal -o esp-backtrace


    // XIAO ESP32-C3 の D7 (GPIO20) に、ログを出したい MCU の TX をつなぐ。
    // 受け取ったバイトをそのまま USB シリアル (/dev/ttyACM*) へ流す UART→USB 変換器。
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let mut uart = Uart::new(peripherals.UART1, UartConfig::default().with_baudrate(115_200))
        .unwrap()
        .with_rx(peripherals.GPIO20);
    let mut usb = UsbSerialJtag::new(peripherals.USB_DEVICE);

    let mut buf = [0u8; 128];
    loop {
        if !uart.read_ready() {
            continue;
        }
        let n = uart.read_buffered(&mut buf).unwrap_or(0);
        for &b in &buf[..n] {
            // PC 側で誰も読んでいなくても詰まらないよう、一定回数で諦めて捨てる。
            for _ in 0..20_000 {
                if usb.write_byte_nb(b).is_ok() {
                    break;
                }
            }
        }
        let _ = usb.flush_tx_nb();
    }
}
