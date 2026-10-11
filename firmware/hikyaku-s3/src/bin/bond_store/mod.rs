//! BLE のボンディング情報（LTK/IRK など）をフラッシュに保存する。
//!
//! パーティションテーブルの `nvs`（0x9000, 24KB）は Wi-Fi を使わないので空いている。
//! その先頭 1 セクタ（4KB）を専用に使う。ボンディングは頻繁には変わらないため、
//! 変更のたびにセクタごと消して書き直す。
//!
//! レイアウト: "HKB1" | count:u8 | (len:u16le | postcard(BondInformation)) × count

use alloc::vec::Vec;
use core::cell::RefCell;
use esp_storage::FlashStorage;
use log::{info, warn};
use trouble_host::prelude::BondInformation;

const OFFSET: u32 = 0x9000;
const SECTOR: usize = 4096;
const MAGIC: [u8; 4] = *b"HKB1";
/// 保持するペア相手の上限。超えたら古いものから捨てる。
const MAX_BONDS: usize = 4;

pub struct BondStore<'a, 'd> {
    /// OTA とフラッシュを共有する。
    flash: &'a RefCell<FlashStorage<'d>>,
    bonds: Vec<BondInformation>,
}

impl<'a, 'd> BondStore<'a, 'd> {
    /// フラッシュから読み出す。壊れている・空の場合は空のストアを返す。
    pub fn load(flash: &'a RefCell<FlashStorage<'d>>) -> Self {
        let mut buf = [0u8; SECTOR];
        let mut bonds = Vec::new();
        if flash.borrow_mut().read(OFFSET, &mut buf).is_ok() && buf[..4] == MAGIC {
            let count = buf[4] as usize;
            let mut pos = 5;
            for _ in 0..count.min(MAX_BONDS) {
                let Some(hdr) = buf.get(pos..pos + 2) else { break };
                let len = u16::from_le_bytes([hdr[0], hdr[1]]) as usize;
                pos += 2;
                let Some(body) = buf.get(pos..pos + len) else { break };
                match postcard::from_bytes::<BondInformation>(body) {
                    Ok(b) => bonds.push(b),
                    Err(_) => {
                        warn!("bond_store: corrupt entry, ignoring the rest");
                        break;
                    }
                }
                pos += len;
            }
        }
        info!("bond_store: loaded {} bond(s)", bonds.len());
        Self { flash, bonds }
    }

    pub fn bonds(&self) -> &[BondInformation] {
        &self.bonds
    }

    /// 同じ相手のものは置き換え、無ければ追加して保存する。
    pub fn save(&mut self, bond: BondInformation) {
        if let Some(i) = self.bonds.iter().position(|b| b.identity.addr == bond.identity.addr) {
            self.bonds[i] = bond;
        } else {
            if self.bonds.len() >= MAX_BONDS {
                self.bonds.remove(0);
            }
            self.bonds.push(bond);
        }
        self.flush();
    }

    fn flush(&mut self) {
        let mut buf = [0xffu8; SECTOR];
        buf[..4].copy_from_slice(&MAGIC);
        buf[4] = self.bonds.len() as u8;
        let mut pos = 5;
        for b in &self.bonds {
            let mut tmp = [0u8; 128];
            let Ok(enc) = postcard::to_slice(b, &mut tmp) else {
                warn!("bond_store: encode failed");
                return;
            };
            buf[pos..pos + 2].copy_from_slice(&(enc.len() as u16).to_le_bytes());
            pos += 2;
            buf[pos..pos + enc.len()].copy_from_slice(enc);
            pos += enc.len();
        }
        let mut flash = self.flash.borrow_mut();
        let res = flash
            .erase(OFFSET, OFFSET + SECTOR as u32)
            .and_then(|_| flash.write(OFFSET, &buf));
        match res {
            Ok(()) => info!("bond_store: saved {} bond(s)", self.bonds.len()),
            Err(_) => warn!("bond_store: flash write failed"),
        }
    }
}
