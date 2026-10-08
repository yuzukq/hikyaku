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
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::usb::otg::{embassy_usb_host::Driver as HostDriver, Usb};

use embassy_futures::select::{select, Either};
use embassy_usb_driver::host::DeviceEvent;
use embassy_usb_host::{
    bus,
    class::kbd::{KbdEvent, KbdHandler},
    handler::{BusRoute, HandlerEvent},
    BusState,
};
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

/// BLE へ流す HID レポート。
#[derive(Clone, Copy)]
enum Report {
    Key([u8; 8]),
    #[allow(dead_code)]
    Mouse([u8; 5]),
    #[allow(dead_code)]
    Consumer([u8; 2]),
}

static REPORTS: Channel<CriticalSectionRawMutex, Report, 32> = Channel::new();

/// USB ホスト：つながったキーボード（ブートプロトコル）を読んで REPORTS に流す。
#[embassy_executor::task]
async fn usb_host(usb: Usb<'static>) {
    static BUS_STATE: BusState = BusState::new();
    // ホストにすると USB シリアル/JTAG（書き込み用）が使えなくなる。
    // 起動後しばらくは猶予を置き、その間なら espflash で書き換えられるようにする。
    Timer::after(Duration::from_secs(3)).await;
    let (mut ctrl, handle) = bus(HostDriver::new(usb), &BUS_STATE);
    let mut cfg = [0u8; 256];

    loop {
        info!("usb: waiting for device");
        let speed = ctrl.wait_for_connection().await;
        let info = match handle.enumerate(BusRoute::Direct(speed), &mut cfg).await {
            Ok((info, _)) => info,
            Err(e) => {
                warn!("usb: enumerate failed: {:?}", e);
                continue;
            }
        };
        info!(
            "usb: device {:04x}:{:04x} addr={}",
            info.device_desc.vendor_id, info.device_desc.product_id, info.device_address
        );

        let mut kbd = match KbdHandler::try_register(&handle, &info).await {
            Ok(k) => k,
            Err(e) => {
                warn!("usb: not a boot keyboard: {:?}", e);
                handle.free_address(info.device_address);
                continue;
            }
        };
        info!("usb: keyboard ready");

        loop {
            match select(kbd.wait_for_event(), ctrl.wait_for_device_event()).await {
                Either::First(Ok(HandlerEvent::HandlerEvent(KbdEvent::KeyStatusUpdate(u)))) => {
                    let mut r = [0u8; 8];
                    r[0] = u.modifiers;
                    for (dst, k) in r[2..].iter_mut().zip(u.keypress.iter()) {
                        *dst = k.map_or(0, |k| k.get());
                    }
                    let _ = REPORTS.try_send(Report::Key(r));
                }
                Either::First(Ok(HandlerEvent::HandlerDisconnected)) => break,
                Either::First(Ok(HandlerEvent::NoChange)) => {}
                Either::First(Err(e)) => {
                    warn!("usb: read error: {:?}", e);
                    break;
                }
                Either::Second(DeviceEvent::Disconnected) => break,
                Either::Second(_) => {}
            }
        }
        info!("usb: keyboard detached");
        handle.free_address(info.device_address);
        // 押しっぱなしのキーが残らないよう全解放を送る。
        let _ = REPORTS.try_send(Report::Key([0; 8]));
    }
}

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
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let usb = Usb::new_fs(peripherals.USB_FS, peripherals.GPIO20, peripherals.GPIO19);
    spawner.spawn(usb_host(usb).unwrap());

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

    info!(
        "handles: kbd={}/{:?} leds={} mouse={}/{:?} consumer={}/{:?}",
        server.hid.keyboard_in.handle,
        server.hid.keyboard_in.cccd_handle,
        server.hid.keyboard_leds.handle,
        server.hid.mouse_in.handle,
        server.hid.mouse_in.cccd_handle,
        server.hid.consumer_in.handle,
        server.hid.consumer_in.cccd_handle,
    );

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
                GattConnectionEvent::Gatt { event } => {
                    match &event {
                        GattEvent::Write(w) => w.with_data(|_, d| info!("gatt write handle={} data={:?}", w.handle(), d)),
                        GattEvent::Read(r) => info!("gatt read handle={}", r.handle()),
                        _ => {}
                    }
                    match event.accept() {
                        Ok(reply) => reply.send().await,
                        Err(e) => warn!("gatt error: {:?}", e),
                    }
                }
                _ => {}
            }
        }
    };
    // 接続前に溜まったレポートは捨てる（古い押下が残って暴走しないように）。
    REPORTS.clear();
    let forward = async {
        if cfg!(feature = "selftest") {
            // `--features selftest`: 5 秒おきに "hello" を打鍵する。
            Timer::after(Duration::from_secs(5)).await;
            loop {
                for key in [0x0b_u8, 0x08, 0x0f, 0x0f, 0x12] {
                    let _ = server.hid.keyboard_in.notify(conn, &[0, 0, key, 0, 0, 0, 0, 0], false).await;
                    Timer::after(Duration::from_millis(30)).await;
                    let _ = server.hid.keyboard_in.notify(conn, &[0; 8], false).await;
                    Timer::after(Duration::from_millis(30)).await;
                }
                Timer::after(Duration::from_secs(5)).await;
            }
        }
        loop {
            let r = REPORTS.receive().await;
            info!("forward report");
            let res = match r {
                Report::Key(v) => server.hid.keyboard_in.notify(conn, &v, false).await,
                Report::Mouse(v) => server.hid.mouse_in.notify(conn, &v, false).await,
                Report::Consumer(v) => server.hid.consumer_in.notify(conn, &v, false).await,
            };
            if let Err(e) = res {
                warn!("notify failed: {:?}", e);
            }
        }
    };
    embassy_futures::select::select(events, forward).await;
}
