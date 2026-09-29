#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use log::{info, warn};
use trouble_host::prelude::*;

use esp_backtrace as _;

extern crate alloc;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 3;
const NAME: &str = "hikyaku";

esp_bootloader_esp_idf::esp_app_desc!();

/// 固定の HID レポート記述子。接続元キーボードによらずこれ 1 つに詰め替える。
///   ID1: キーボード (mods, reserved, keys[6]) + LED 出力
///   ID2: マウス (buttons, x, y, wheel, pan)
///   ID3: コンシューマ (16bit usage)
#[rustfmt::skip]
const REPORT_MAP: [u8; 149] = [
    // Keyboard
    0x05,0x01, 0x09,0x06, 0xA1,0x01, 0x85,0x01,
    0x05,0x07, 0x19,0xE0, 0x29,0xE7, 0x15,0x00, 0x25,0x01, 0x75,0x01, 0x95,0x08, 0x81,0x02,
    0x95,0x01, 0x75,0x08, 0x81,0x01,
    0x95,0x05, 0x75,0x01, 0x05,0x08, 0x19,0x01, 0x29,0x05, 0x91,0x02,
    0x95,0x01, 0x75,0x03, 0x91,0x01,
    0x95,0x06, 0x75,0x08, 0x15,0x00, 0x26,0xFF,0x00, 0x05,0x07, 0x19,0x00, 0x2A,0xFF,0x00, 0x81,0x00,
    0xC0,
    // Mouse
    0x05,0x01, 0x09,0x02, 0xA1,0x01, 0x85,0x02, 0x09,0x01, 0xA1,0x00,
    0x05,0x09, 0x19,0x01, 0x29,0x08, 0x15,0x00, 0x25,0x01, 0x95,0x08, 0x75,0x01, 0x81,0x02,
    0x05,0x01, 0x09,0x30, 0x09,0x31, 0x09,0x38, 0x15,0x81, 0x25,0x7F, 0x75,0x08, 0x95,0x03, 0x81,0x06,
    0x05,0x0C, 0x0A,0x38,0x02, 0x95,0x01, 0x81,0x06,
    0xC0, 0xC0,
    // Consumer
    0x05,0x0C, 0x09,0x01, 0xA1,0x01, 0x85,0x03, 0x19,0x00, 0x2A,0xA0,0x02, 0x15,0x00, 0x26,0xA0,0x02,
    0x75,0x10, 0x95,0x01, 0x81,0x00, 0xC0,
];

#[gatt_server]
struct Server {
    hid: HidService,
    battery: BatteryService,
}

#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
struct HidService {
    #[characteristic(uuid = "2a4a", read, value = [0x11, 0x01, 0x00, 0x02])]
    hid_info: [u8; 4],
    #[characteristic(uuid = "2a4b", read, value = REPORT_MAP)]
    report_map: [u8; 149],
    #[characteristic(uuid = "2a4c", write_without_response)]
    control_point: u8,
    #[characteristic(uuid = "2a4e", read, write_without_response, value = 1)]
    protocol_mode: u8,
    #[descriptor(uuid = "2908", read, value = [0x01, 0x01])]
    #[characteristic(uuid = "2a4d", read, notify, value = [0; 8])]
    keyboard_in: [u8; 8],
    #[descriptor(uuid = "2908", read, value = [0x01, 0x02])]
    #[characteristic(uuid = "2a4d", read, write, write_without_response, value = [0])]
    keyboard_leds: [u8; 1],
    #[descriptor(uuid = "2908", read, value = [0x02, 0x01])]
    #[characteristic(uuid = "2a4d", read, notify, value = [0; 5])]
    mouse_in: [u8; 5],
    #[descriptor(uuid = "2908", read, value = [0x03, 0x01])]
    #[characteristic(uuid = "2a4d", read, notify, value = [0; 2])]
    consumer_in: [u8; 2],
}

#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 100)]
    level: u8,
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let transport = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let controller = ExternalController::<_, 1>::new(transport);
    let mut resources: HostResources<_, DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();

    let address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .set_io_capabilities(IoCapabilities::NoInputNoOutput)
        .build();
    let mut peripheral = stack.peripheral();
    let mut runner = stack.runner();

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: NAME,
        appearance: &appearance::human_interface_device::KEYBOARD,
    }))
    .unwrap();

    let app = async {
        loop {
            match advertise(&mut peripheral, &server).await {
                Ok(conn) => {
                    info!("connected");
                    let _ = conn.raw().request_security();
                    session(&server, &conn).await;
                    info!("disconnected");
                }
                Err(e) => {
                    warn!("advertise error: {:?}", e);
                    Timer::after(Duration::from_secs(1)).await;
                }
            }
        }
    };
    let _ = join(runner.run(), app).await;
    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

async fn advertise<'v, 's, C: Controller>(
    peripheral: &mut Peripheral<'v, C, DefaultPacketPool>,
    server: &'s Server<'v>,
) -> Result<GattConnection<'v, 's, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut adv = [0u8; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteServiceUuids16(&[[0x12, 0x18]]),
            AdStructure::CompleteLocalName(NAME.as_bytes()),
        ],
        &mut adv[..],
    )?;
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected { adv_data: &adv[..len], scan_data: &[] },
        )
        .await?;
    info!("advertising as {}", NAME);
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    Ok(conn)
}

/// 接続中の処理。GATT イベントを捌きつつ、テスト用に定期的に "hello" を打鍵する。
async fn session<P: PacketPool>(server: &Server<'_>, conn: &GattConnection<'_, '_, P>) {
    let events = async {
        loop {
            match conn.next().await {
                GattConnectionEvent::Disconnected { reason } => {
                    info!("disconnect: {:?}", reason);
                    break;
                }
                GattConnectionEvent::PairingComplete { security_level, bond } => {
                    info!("pairing complete: {:?} bonded={}", security_level, bond.is_some());
                }
                GattConnectionEvent::PairingFailed(e) => warn!("pairing failed: {:?}", e),
                GattConnectionEvent::Gatt { event } => match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("gatt error: {:?}", e),
                },
                _ => {}
            }
        }
    };
    let typing = async {
        Timer::after(Duration::from_secs(5)).await;
        loop {
            // h e l l o
            for key in [0x0b_u8, 0x08, 0x0f, 0x0f, 0x12] {
                let _ = server.hid.keyboard_in.notify(conn, &[0, 0, key, 0, 0, 0, 0, 0], false).await;
                Timer::after(Duration::from_millis(30)).await;
                let _ = server.hid.keyboard_in.notify(conn, &[0; 8], false).await;
                Timer::after(Duration::from_millis(30)).await;
            }
            Timer::after(Duration::from_secs(5)).await;
        }
    };
    embassy_futures::select::select(events, typing).await;
}
