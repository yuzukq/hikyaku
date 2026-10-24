//! BLE のボンディング情報（LTK/IRK など）を、送り先（プロファイル）ごとにフラッシュに保存する。
//!
//! パーティションテーブルの `nvs`（0x9000, 24KB）は Wi-Fi を使わないので空いている。
//! その先頭 1 セクタ（4KB）を専用に使う。ボンディングは頻繁には変わらないため、
//! 変更のたびにセクタごと消して書き直す。
//!
//! レイアウト: "HKB2" | count:u8 | (profile:u8 | len:u16le | postcard(BondInformation)) × count
//! 古い "HKB1"（送り先の区別なし。2026-10-23 まで）は、すべて 1 台目（profile 0）のものとして読む。

use alloc::vec::Vec;
use core::cell::RefCell;
use esp_storage::FlashStorage;
use log::{info, warn};
use trouble_host::prelude::BondInformation;

const OFFSET: u32 = 0x9000;
const SECTOR: usize = 4096;
const MAGIC: [u8; 4] = *b"HKB2";
const MAGIC_V1: [u8; 4] = *b"HKB1";
/// 送り先ごとに保持するペア相手の上限。超えたら、その送り先の古いものから捨てる。
const MAX_BONDS_PER_PROFILE: usize = 3;

pub struct BondStore<'a, 'd> {
    /// OTA とフラッシュを共有する。
    flash: &'a RefCell<FlashStorage<'d>>,
    /// 今の送り先。新しいボンドはこの送り先のものとして保存し、`bonds()` もこの送り先のものだけを返す。
    profile: u8,
    /// (送り先, ボンド)。ほかの送り先のものも、書き直すときに消さないよう持っておく。
    entries: Vec<(u8, BondInformation)>,
}

impl<'a, 'd> BondStore<'a, 'd> {
    /// フラッシュから読み出す。壊れている・空の場合は空のストアを返す。
    pub fn load(flash: &'a RefCell<FlashStorage<'d>>, profile: u8) -> Self {
        let mut buf = [0u8; SECTOR];
        let mut entries = Vec::new();
        if flash.borrow_mut().read(OFFSET, &mut buf).is_ok() && (buf[..4] == MAGIC || buf[..4] == MAGIC_V1) {
            let v1 = buf[..4] == MAGIC_V1;
            let count = buf[4] as usize;
            let mut pos = 5;
            for _ in 0..count.min(16) {
                let owner = if v1 {
                    0
                } else {
                    let Some(&p) = buf.get(pos) else { break };
                    pos += 1;
                    p
                };
                let Some(hdr) = buf.get(pos..pos + 2) else { break };
                let len = u16::from_le_bytes([hdr[0], hdr[1]]) as usize;
                pos += 2;
                let Some(body) = buf.get(pos..pos + len) else { break };
                match postcard::from_bytes::<BondInformation>(body) {
                    Ok(b) => entries.push((owner, b)),
                    Err(_) => {
                        warn!("bond_store: corrupt entry, ignoring the rest");
                        break;
                    }
                }
                pos += len;
            }
        }
        let store = Self { flash, profile, entries };
        info!("bond_store: loaded {} bond(s), {} for profile {}", store.entries.len(), store.bonds().count(), profile + 1);
        store
    }

    /// 今の送り先のボンド。
    pub fn bonds(&self) -> impl Iterator<Item = &BondInformation> {
        self.entries.iter().filter(|(p, _)| *p == self.profile).map(|(_, b)| b)
    }

    /// 今の送り先の同じ相手のものは置き換え、無ければ追加して保存する。
    pub fn save(&mut self, bond: BondInformation) {
        let profile = self.profile;
        if let Some(e) = self.entries.iter_mut().find(|(p, b)| *p == profile && b.identity.addr == bond.identity.addr) {
            e.1 = bond;
        } else {
            if self.bonds().count() >= MAX_BONDS_PER_PROFILE {
                if let Some(i) = self.entries.iter().position(|(p, _)| *p == profile) {
                    self.entries.remove(i);
                }
            }
            self.entries.push((profile, bond));
        }
        self.flush();
    }

    fn flush(&mut self) {
        let mut buf = [0xffu8; SECTOR];
        buf[..4].copy_from_slice(&MAGIC);
        let mut pos = 5;
        let mut count = 0u8;
        for (p, b) in &self.entries {
            let mut tmp = [0u8; 128];
            let Ok(enc) = postcard::to_slice(b, &mut tmp) else {
                warn!("bond_store: encode failed");
                return;
            };
            if pos + 3 + enc.len() > SECTOR {
                warn!("bond_store: sector full, dropping the rest");
                break;
            }
            buf[pos] = *p;
            buf[pos + 1..pos + 3].copy_from_slice(&(enc.len() as u16).to_le_bytes());
            pos += 3;
            buf[pos..pos + enc.len()].copy_from_slice(enc);
            pos += enc.len();
            count += 1;
        }
        buf[4] = count;
        let mut flash = self.flash.borrow_mut();
        let res = flash
            .erase(OFFSET, OFFSET + SECTOR as u32)
            .and_then(|_| flash.write(OFFSET, &buf));
        match res {
            Ok(()) => info!("bond_store: saved {} bond(s)", count),
            Err(_) => warn!("bond_store: flash write failed"),
        }
    }
}
