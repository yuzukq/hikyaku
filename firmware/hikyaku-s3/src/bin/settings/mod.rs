//! 本体の設定（今は置き方だけ）をフラッシュに保存する。
//!
//! `nvs` パーティション（0x9000, 24KB。Wi-Fi を使わないので空いている）の 2 セクタ目（0xA000、4KB）を使う。
//! 1 セクタ目はボンディング情報（`bond_store`）。
//!
//! レイアウト: "HKS1" | rotation:u8（時計回りに 90° を何回か）

use core::cell::RefCell;
use esp_storage::FlashStorage;
use hikyaku_pointer::Rotation;
use log::{info, warn};

const OFFSET: u32 = 0xA000;
const SECTOR: u32 = 4096;
const MAGIC: [u8; 4] = *b"HKS1";

/// 保存されている置き方（なければ・壊れていれば基準の向き）。
pub fn load_rotation(flash: &RefCell<FlashStorage<'_>>) -> Rotation {
    let mut buf = [0u8; 8];
    let r = match flash.borrow_mut().read(OFFSET, &mut buf) {
        Ok(()) if buf[..4] == MAGIC => Rotation::from_quarters(buf[4]).unwrap_or_default(),
        _ => Rotation::R0,
    };
    info!("settings: rotation {}°", r.quarters() as u16 * 90);
    r
}

pub fn save_rotation(flash: &RefCell<FlashStorage<'_>>, r: Rotation) {
    let mut buf = [0xffu8; 8];
    buf[..4].copy_from_slice(&MAGIC);
    buf[4] = r.quarters();
    let mut flash = flash.borrow_mut();
    match flash.erase(OFFSET, OFFSET + SECTOR).and_then(|_| flash.write(OFFSET, &buf)) {
        Ok(()) => info!("settings: saved rotation {}°", r.quarters() as u16 * 90),
        Err(_) => warn!("settings: flash write failed"),
    }
}
