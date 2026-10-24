//! 本体の設定（置き方・送り先・送り先ごとのスクロールの向き）をフラッシュに保存する。
//!
//! `nvs` パーティション（0x9000, 24KB。Wi-Fi を使わないので空いている）の 2 セクタ目（0xA000、4KB）を使う。
//! 1 セクタ目はボンディング情報（`bond_store`）。
//!
//! レイアウト: "HKS2" | rotation:u8（時計回りに 90° を何回か） | profile:u8（送り先。0 = 1 台目） |
//! scroll_invert:u8（bit n = 送り先 n でスクロールを反転）。
//! 古い "HKS1" | rotation（2026-10-23）も読む（送り先は 1 台目、スクロールの向きは既定）。

use core::cell::{Cell, RefCell};
use critical_section::Mutex;
use esp_storage::FlashStorage;
use log::{info, warn};

const OFFSET: u32 = 0xA000;
const SECTOR: u32 = 4096;
const MAGIC: [u8; 4] = *b"HKS2";
const MAGIC_V1: [u8; 4] = *b"HKS1";

/// 送り先の数（docs/design/multi-host.md）。
pub const PROFILES: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// 本体の置き方（時計回りに 90° を何回か、0〜3。`hikyaku_pointer::Rotation`）
    pub rotation: u8,
    /// 今の送り先（0 = 1 台目 hikyaku、1 = 2 台目 hikyaku2）
    pub profile: u8,
    /// bit n = 送り先 n でスクロールを反転する
    pub scroll_invert: u8,
}

impl Settings {
    /// 既定: 1 台目（Mac）はスクロールを反転（Mac の「ナチュラル」に合わせて 2026-10-20 に決めた向き）、2 台目（Windows）は反転しない。
    pub const DEFAULT: Settings = Settings { rotation: 0, profile: 0, scroll_invert: 0b01 };

    /// 今の送り先でスクロールを反転するか。
    pub fn scroll_invert_now(&self) -> bool {
        self.scroll_invert >> self.profile & 1 != 0
    }
}

static CURRENT: Mutex<Cell<Settings>> = Mutex::new(Cell::new(Settings::DEFAULT));

/// 今の設定。
pub fn get() -> Settings {
    critical_section::with(|cs| CURRENT.borrow(cs).get())
}

/// 起動時に読む（なければ・壊れていれば既定）。
pub fn load(flash: &RefCell<FlashStorage<'_>>) -> Settings {
    let mut buf = [0u8; 8];
    let mut s = Settings::DEFAULT;
    if flash.borrow_mut().read(OFFSET, &mut buf).is_ok() {
        if buf[..4] == MAGIC {
            s = Settings { rotation: buf[4] & 3, profile: buf[5].min(PROFILES - 1), scroll_invert: buf[6] };
        } else if buf[..4] == MAGIC_V1 {
            s.rotation = buf[4] & 3;
        }
    }
    critical_section::with(|cs| CURRENT.borrow(cs).set(s));
    info!("settings: rotation {}°, profile {}, scroll invert {:#04b}", s.rotation as u16 * 90, s.profile + 1, s.scroll_invert);
    s
}

/// 変えて保存する（変わらなければ書かない）。変えた後の設定を返す。
pub fn update(flash: &RefCell<FlashStorage<'_>>, f: impl FnOnce(&mut Settings)) -> Settings {
    let old = get();
    let mut s = old;
    f(&mut s);
    if s != old {
        critical_section::with(|cs| CURRENT.borrow(cs).set(s));
        save(flash, &s);
    }
    s
}

fn save(flash: &RefCell<FlashStorage<'_>>, s: &Settings) {
    let mut buf = [0xffu8; 8];
    buf[..4].copy_from_slice(&MAGIC);
    buf[4] = s.rotation;
    buf[5] = s.profile;
    buf[6] = s.scroll_invert;
    let mut flash = flash.borrow_mut();
    match flash.erase(OFFSET, OFFSET + SECTOR).and_then(|_| flash.write(OFFSET, &buf)) {
        Ok(()) => info!("settings: saved {:?}", s),
        Err(_) => warn!("settings: flash write failed"),
    }
}
