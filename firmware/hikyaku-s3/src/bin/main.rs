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
use embassy_sync::signal::Signal;
use embassy_time::{with_timeout, Duration, Timer};
use bond_store::BondStore;
use core::cell::RefCell;
use esp_hal::clock::CpuClock;
use esp_storage::FlashStorage;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::usb::otg::{embassy_usb_host::Driver as HostDriver, Usb};

use embassy_futures::select::{select, select3, Either, Either3};
use embassy_usb_driver::host::{pipe, DeviceEvent, UsbHostAllocator, UsbPipe};
use embassy_usb_driver::{EndpointInfo, EndpointType};
use embassy_usb_host::{
    bus,
    class::kbd::{KbdEvent, KbdHandler},
    control::SetupPacket,
    handler::{BusRoute, EnumerationInfo, HandlerEvent},
    BusState,
};
use esp_radio::ble::controller::BleConnector;
use log::{debug, info, warn};
use trouble_host::prelude::*;

use esp_backtrace as _;

extern crate alloc;

mod bond_store;
mod ota;
#[cfg(feature = "trackpad")]
mod trackpad;

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
    Mouse([u8; 5]),
    Consumer([u8; 2]),
}

static REPORTS: Channel<CriticalSectionRawMutex, Report, 32> = Channel::new();

/// キーボードの組み合わせ（Ctrl+Alt+Shift+Gui+Enter+U）で OTA モードに入る要求。
static OTA_REQUEST: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// 修飾キーに Ctrl・Shift・Alt・Gui（左右どちらでも）がそろい、Enter(0x28) と U(0x18) が押されている。
fn is_ota_chord(modifiers: u8, keys: &[Option<core::num::NonZeroU8>; 6]) -> bool {
    let hyper = modifiers & 0x11 != 0 && modifiers & 0x22 != 0 && modifiers & 0x44 != 0 && modifiers & 0x88 != 0;
    let has = |code: u8| keys.iter().any(|k| k.is_some_and(|k| k.get() == code));
    hyper && has(0x28) && has(0x18)
}


/// キーボードの「マウス + コンシューマ（音量など）」インターフェースを探して、
/// その割り込み IN パイプを開く。レポート記述子の先頭が Generic Desktop / Mouse のものを選ぶ
/// （QMK の raw HID など、他の HID インターフェースと区別するため）。
/// ブートキーボード側で SET_CONFIGURATION 済みである前提（ここでは設定し直さない）。
async fn open_media_pipe<'d, A: UsbHostAllocator<'d>>(
    alloc: &A,
    info: &EnumerationInfo,
) -> Option<A::Pipe<pipe::Interrupt, pipe::In>> {
    let mut ctrl = alloc
        .alloc_pipe::<pipe::Control, pipe::InOut>(
            info.device_address,
            &EndpointInfo {
                addr: 0.into(),
                ep_type: EndpointType::Control,
                max_packet_size: (info.device_desc.max_packet_size0 as u16).min(info.speed().max_packet_size()),
                interval_ms: 0,
            },
            info.split(),
        )
        .ok()?;

    let mut cfg_buf = [0u8; 512];
    let cfg = info.active_config_or_set_default(&mut ctrl, &mut cfg_buf).await.ok()?;

    for iface in cfg.iter_interface() {
        // HID クラスで、ブートキーボード(subclass 1 / protocol 1)以外。
        if iface.interface_class != 0x03 || (iface.interface_subclass == 1 && iface.interface_protocol == 1) {
            continue;
        }
        let desc_len = iface.iter_descriptors().find_map(|(_, d)| {
            (d.len() >= 7 && d[1] == 0x21).then(|| u16::from_le_bytes([d[5], d[6]]))
        })?;
        let mut rd = [0u8; 256];
        let len = (desc_len as usize).min(rd.len());
        let setup = SetupPacket::get_hid_report_descriptor(iface.interface_number, len as u16);
        let n = ctrl.control_in(&setup.to_bytes(), &mut rd[..len]).await.ok()?;
        if !rd[..n].starts_with(&[0x05, 0x01, 0x09, 0x02]) {
            continue;
        }
        let ep = iface
            .iter_endpoints()
            .find(|e| e.ep_type() == EndpointType::Interrupt && e.ep_dir() == embassy_usb_driver::Direction::In)?;
        info!("usb: media interface {} (report desc {} bytes)", iface.interface_number, n);
        return alloc
            .alloc_pipe::<pipe::Interrupt, pipe::In>(info.device_address, &ep.into(), info.split())
            .ok();
    }
    None
}

/// IF2 のレポート（先頭 1 バイトがレポート ID）を BLE 用のレポートに変換する。
///   ID2: マウス (buttons, x, y, wheel, pan) → そのまま
///   ID4: コンシューマ (usage 16bit)        → こちらの ID3 へ
///   ID3: システムコントロール（電源/スリープ）は BLE 側の記述子に無いので捨てる
fn media_report(buf: &[u8]) -> Option<Report> {
    match (buf.first()?, buf.len()) {
        (2, 6) => Some(Report::Mouse([buf[1], buf[2], buf[3], buf[4], buf[5]])),
        (4, 3) => Some(Report::Consumer([buf[1], buf[2]])),
        _ => None,
    }
}

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

        let mut media = open_media_pipe(&handle, &info).await;
        if media.is_none() {
            info!("usb: no media interface (volume/mouse keys will not be forwarded)");
        }
        let mut mbuf = [0u8; 16];

        loop {
            let media_in = async {
                match media.as_mut() {
                    Some(pipe) => pipe.request_in(&mut mbuf).await,
                    None => core::future::pending().await,
                }
            };
            match select(select(kbd.wait_for_event(), media_in), ctrl.wait_for_device_event()).await {
                Either::First(Either::Second(Ok(n))) => {
                    if let Some(r) = media_report(&mbuf[..n]) {
                        let _ = REPORTS.try_send(r);
                    }
                }
                Either::First(Either::Second(Err(e))) => {
                    warn!("usb: media read error: {:?}", e);
                    break;
                }
                Either::First(Either::First(Ok(HandlerEvent::HandlerEvent(KbdEvent::KeyStatusUpdate(u))))) => {
                    if is_ota_chord(u.modifiers, &u.keypress) {
                        info!("usb: OTA chord");
                        OTA_REQUEST.signal(());
                        let _ = REPORTS.try_send(Report::Key([0; 8])); // 押しっぱなしを残さない。転送はしない
                        continue;
                    }
                    let mut r = [0u8; 8];
                    r[0] = u.modifiers;
                    for (dst, k) in r[2..].iter_mut().zip(u.keypress.iter()) {
                        *dst = k.map_or(0, |k| k.get());
                    }
                    let _ = REPORTS.try_send(Report::Key(r));
                }
                Either::First(Either::First(Ok(HandlerEvent::HandlerDisconnected))) => break,
                Either::First(Either::First(Ok(HandlerEvent::NoChange))) => {}
                Either::First(Either::First(Err(e))) => {
                    warn!("usb: read error: {:?}", e);
                    break;
                }
                Either::Second(DeviceEvent::Disconnected) => break,
                Either::Second(_) => {}
            }
        }
        info!("usb: keyboard detached");
        handle.free_address(info.device_address);
        // 押しっぱなしが残らないよう全解放を送る。
        let _ = REPORTS.try_send(Report::Key([0; 8]));
        let _ = REPORTS.try_send(Report::Mouse([0; 5]));
        let _ = REPORTS.try_send(Report::Consumer([0; 2]));
    }
}

#[gatt_server]
struct Server {
    hid: HidService,
    battery: BatteryService,
    /// OTA（常に存在するが、OTA モード以外では書き込みを拒否する）。末尾に置いて、既存サービスのハンドルを動かさない。
    ota: OtaService,
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

#[gatt_service(uuid = "6b9a0001-6f1e-4b8e-9d57-7f0d0a5b0001")]
struct OtaService {
    #[characteristic(uuid = "6b9a0002-6f1e-4b8e-9d57-7f0d0a5b0001", write, notify, value = [0; 80])]
    ctrl: [u8; 80],
    #[characteristic(uuid = "6b9a0003-6f1e-4b8e-9d57-7f0d0a5b0001", write_without_response, value = [0; 244])]
    data: [u8; 244],
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

    #[cfg(feature = "trackpad")]
    {
        use esp_hal::{
            gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
            spi::{master::{Config as SpiConfig, Spi}, Mode},
            time::Rate,
        };
        let spi = Spi::new(
            peripherals.SPI2,
            SpiConfig::default().with_frequency(Rate::from_mhz(1)).with_mode(Mode::_1),
        )
        .unwrap()
        .with_sck(peripherals.GPIO12)
        .with_mosi(peripherals.GPIO11)
        .with_miso(peripherals.GPIO13);
        let cs = Output::new(peripherals.GPIO10, Level::High, OutputConfig::default());
        let dr = Input::new(peripherals.GPIO9, InputConfig::default().with_pull(Pull::Down));
        spawner.spawn(trackpad::run(spi, cs, dr).unwrap());
    }

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
    // 保存済みのペア情報を復元する（Mac 側で削除しない限り、再起動後も再ペアリング不要）。
    let flash = RefCell::new(FlashStorage::new(peripherals.FLASH));
    ota::mark_app_valid(&flash);
    let store = RefCell::new(BondStore::load(&flash));
    for bond in store.borrow().bonds() {
        if let Err(e) = stack.add_bond_information(bond.clone()) {
            warn!("restore bond failed: {:?}", e);
        }
    }
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
            let conn = match select(advertise(&mut peripheral, &server, NAME, &Default::default()), OTA_REQUEST.wait()).await {
                Either::First(Ok(conn)) => conn,
                Either::First(Err(e)) => {
                    warn!("advertise error: {:?}", e);
                    Timer::after(Duration::from_secs(1)).await;
                    continue;
                }
                Either::Second(()) => ota_mode(&mut peripheral, &server, &flash).await,
            };
            info!("connected");
            // ボンディングを許可（これが無いと PairingComplete の bond が None になり保存されない）。
            let _ = conn.raw().set_bondable(true);
            let _ = conn.raw().request_security();
            if let SessionEnd::Ota = session(&server, &conn, &store).await {
                drop(conn);
                ota_mode(&mut peripheral, &server, &flash).await;
            }
            info!("disconnected");
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
    name: &str,
    params: &AdvertisementParameters,
) -> Result<GattConnection<'v, 's, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut adv = [0u8; 31];
    let len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteServiceUuids16(&[[0x12, 0x18]]),
            AdStructure::CompleteLocalName(name.as_bytes()),
        ],
        &mut adv[..],
    )?;
    let advertiser = peripheral
        .advertise(
            params,
            Advertisement::ConnectableScannableUndirected { adv_data: &adv[..len], scan_data: &[] },
        )
        .await?;
    info!("advertising as {}", name);
    let conn = advertiser.accept().await?.with_attribute_server(server)?;
    Ok(conn)
}

/// 接続中の処理。GATT イベントを捌きつつ、USB キーボードのレポートを BLE へ流す。
async fn session<P: PacketPool>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, P>,
    store: &RefCell<BondStore<'_, '_>>,
) -> SessionEnd {
    let events = async {
        loop {
            match conn.next().await {
                GattConnectionEvent::Disconnected { reason } => {
                    info!("disconnect: {:?}", reason);
                    break;
                }
                GattConnectionEvent::PairingComplete { security_level, bond } => {
                    info!("pairing complete: {:?} bonded={}", security_level, bond.is_some());
                    if let Some(bond) = bond {
                        store.borrow_mut().save(bond);
                    }
                }
                GattConnectionEvent::PairingFailed(e) => warn!("pairing failed: {:?}", e),
                GattConnectionEvent::Gatt { event } => {
                    // OTA の書き込みは OTA モード以外では受け付けない。
                    let is_ota = matches!(&event, GattEvent::Write(w)
                        if w.handle() == server.ota.ctrl.handle || w.handle() == server.ota.data.handle);
                    if is_ota {
                        warn!("ota: write rejected (not in OTA mode)");
                        match event.reject(AttErrorCode::WRITE_NOT_PERMITTED) {
                            Ok(reply) => reply.send().await,
                            Err(e) => warn!("gatt error: {:?}", e),
                        }
                        continue;
                    }
                    match &event {
                        GattEvent::Write(w) => w.with_data(|_, d| debug!("gatt write handle={} data={:?}", w.handle(), d)),
                        GattEvent::Read(r) => debug!("gatt read handle={}", r.handle()),
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
            debug!("forward report");
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
    match select3(events, forward, OTA_REQUEST.wait()).await {
        Either3::Third(()) => SessionEnd::Ota,
        _ => SessionEnd::Disconnected,
    }
}

enum SessionEnd {
    Disconnected,
    /// キーボードから OTA モードが要求された。
    Ota,
}

enum Action {
    None,
    Notify([u8; ota::CTRL_LEN]),
    /// 通知を送ってから再起動する。
    Reboot([u8; ota::CTRL_LEN]),
}

/// OTA モード。ホストとの接続は切り、`hikyaku-OTA` として PC からの接続を待つ。終わったら（または時間切れで）再起動する。
async fn ota_mode<'v, C: Controller>(
    peripheral: &mut Peripheral<'v, C, DefaultPacketPool>,
    server: &Server<'v>,
    flash: &RefCell<FlashStorage<'_>>,
) -> ! {
    match ota::key() {
        None => warn!("ota: HIKYAKU_OTA_KEY was not set at build time; OTA is disabled"),
        Some(key) => {
            info!("ota: mode on for 180 s (advertising as hikyaku-OTA)");
            let mut ota = ota::Ota::new(flash, key);
            let r = with_timeout(Duration::from_secs(180), ota_session(peripheral, server, &mut ota)).await;
            info!("ota: {}", if r.is_ok() { "finished" } else { "timed out" });
        }
    }
    Timer::after(Duration::from_millis(300)).await;
    esp_hal::system::software_reset()
}

async fn ota_session<'v, C: Controller>(
    peripheral: &mut Peripheral<'v, C, DefaultPacketPool>,
    server: &Server<'v>,
    ota: &mut ota::Ota<'_, '_>,
) {
    let (ctrl_h, data_h) = (server.ota.ctrl.handle, server.ota.data.handle);
    // 接続を受ける相手を PC に限る（指定がなければ誰でも。HMAC 認証は常に必要）。
    let mut params = AdvertisementParameters::default();
    if let Some(peer) = ota::peer() {
        let addr = Address::new(AddrKind::PUBLIC, BdAddr::new(peer));
        match peripheral.set_filter_accept_list(&[addr]).await {
            Ok(()) => {
                params.filter_policy = AdvFilterPolicy::FilterConn;
                info!("ota: accepting connections only from the configured PC");
            }
            Err(_) => warn!("ota: could not set the accept list"),
        }
    } else {
        warn!("ota: HIKYAKU_OTA_PEER not set; accepting any central");
    }
    loop {
        let conn = match advertise(peripheral, server, "hikyaku-OTA", &params).await {
            Ok(c) => c,
            Err(e) => {
                warn!("ota: advertise error: {:?}", e);
                Timer::after(Duration::from_secs(1)).await;
                continue;
            }
        };
        info!("ota: connected");
        loop {
            match conn.next().await {
                GattConnectionEvent::Disconnected { .. } => break,
                GattConnectionEvent::Gatt { event } => {
                    let action = match &event {
                        GattEvent::Write(w) if w.handle() == ctrl_h => w.with_data(|_, d| match ota.ctrl(d) {
                            ota::Reply::None => Action::None,
                            ota::Reply::Notify(n) => Action::Notify(n),
                            ota::Reply::NotifyThenReboot(n) => Action::Reboot(n),
                        }),
                        GattEvent::Write(w) if w.handle() == data_h => w.with_data(|_, d| match ota.data(d) {
                            Ok(Some(off)) => Action::Notify(ota::Ota::progress_note(off)),
                            Ok(None) => Action::None,
                            Err(st) => {
                                warn!("ota: data error: {:?}", st);
                                Action::Notify(ota::Ota::error_note(st))
                            }
                        }),
                        _ => Action::None,
                    };
                    match event.accept() {
                        Ok(reply) => reply.send().await,
                        Err(e) => warn!("gatt error: {:?}", e),
                    }
                    match action {
                        Action::None => {}
                        Action::Notify(n) => {
                            let _ = server.ota.ctrl.notify(&conn, &n, false).await;
                        }
                        Action::Reboot(n) => {
                            let _ = server.ota.ctrl.notify(&conn, &n, false).await;
                            Timer::after(Duration::from_millis(500)).await;
                            return;
                        }
                    }
                }
                _ => {}
            }
        }
        info!("ota: disconnected, waiting again");
    }
}
