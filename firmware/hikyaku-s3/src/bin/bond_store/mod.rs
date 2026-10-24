//! BLE のボンディング情報（LTK/IRK など）と、相手ごとの購読の状態（CCCD）を、送り先（プロファイル）ごとにフラッシュに保存する。
//!
//! パーティションテーブルの `nvs`（0x9000, 24KB）は Wi-Fi を使わないので空いている。
//! その先頭 1 セクタ（4KB）を専用に使う。ボンディングは頻繁には変わらないため、
//! 変更のたびにセクタごと消して書き直す。
//!
//! レイアウト: "HKB3" | count:u8 | (profile:u8 | len:u16le | postcard(BondInformation) | cccd_len:u8 | cccd) × count
//! 古い "HKB2"（購読の状態なし。2026-10-24 まで）、"HKB1"（送り先の区別もなし。すべて 1 台目）も読む。
//!
//! 購読の状態: ボンド済みの相手は、つなぎ直しても CCCD を書き直さない（サーバが覚えている前提。Windows がそう）。
//! trouble-host は RAM にしか覚えないので、hikyaku が再起動すると通知が届かなくなる。そこで `ClientAttTable` の
//! バイト列をボンドと一緒に保存し、再起動後の最初の接続で戻す（main.rs の `session`）。

use alloc::vec::Vec;
use core::cell::RefCell;
use esp_storage::FlashStorage;
use log::{info, warn};
use trouble_host::prelude::{BondInformation, Identity};

const OFFSET: u32 = 0x9000;
const SECTOR: usize = 4096;
const MAGIC: [u8; 4] = *b"HKB3";
const MAGIC_V2: [u8; 4] = *b"HKB2";
const MAGIC_V1: [u8; 4] = *b"HKB1";
/// 送り先ごとに保持するペア相手の上限。超えたら、その送り先の古いものから捨てる。
const MAX_BONDS_PER_PROFILE: usize = 3;

struct Entry {
    profile: u8,
    bond: BondInformation,
    /// 購読の状態（`ClientAttTable::raw()`。trouble-host の CLIENT_ATT_TABLE_SIZE = 64 バイト以内）。空なら未保存
    cccd: Vec<u8>,
}

pub struct BondStore<'a, 'd> {
    /// OTA とフラッシュを共有する。
    flash: &'a RefCell<FlashStorage<'d>>,
    /// 今の送り先。新しいボンドはこの送り先のものとして保存し、`bonds()` もこの送り先のものだけを返す。
    profile: u8,
    /// ほかの送り先のものも、書き直すときに消さないよう持っておく。
    entries: Vec<Entry>,
    /// この起動で、もう購読の状態を戻した相手（2 回目以降の接続では RAM の状態を正とする）
    restored: Vec<Identity>,
}

impl<'a, 'd> BondStore<'a, 'd> {
    /// フラッシュから読み出す。壊れている・空の場合は空のストアを返す。
    pub fn load(flash: &'a RefCell<FlashStorage<'d>>, profile: u8) -> Self {
        let mut buf = [0u8; SECTOR];
        let mut entries = Vec::new();
        let ok = flash.borrow_mut().read(OFFSET, &mut buf).is_ok();
        let magic = [buf[0], buf[1], buf[2], buf[3]];
        if ok && (magic == MAGIC || magic == MAGIC_V2 || magic == MAGIC_V1) {
            let count = buf[4] as usize;
            let mut pos = 5;
            for _ in 0..count.min(16) {
                let owner = if magic == MAGIC_V1 {
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
                let bond = match postcard::from_bytes::<BondInformation>(body) {
                    Ok(b) => b,
                    Err(_) => {
                        warn!("bond_store: corrupt entry, ignoring the rest");
                        break;
                    }
                };
                pos += len;
                let mut cccd = Vec::new();
                if magic == MAGIC {
                    let Some(&n) = buf.get(pos) else { break };
                    pos += 1;
                    let Some(bytes) = buf.get(pos..pos + n as usize) else { break };
                    cccd.extend_from_slice(bytes);
                    pos += n as usize;
                }
                entries.push(Entry { profile: owner, bond, cccd });
            }
        }
        let store = Self { flash, profile, entries, restored: Vec::new() };
        info!("bond_store: loaded {} bond(s), {} for profile {}", store.entries.len(), store.bonds().count(), profile + 1);
        store
    }

    /// 今の送り先のボンド。
    pub fn bonds(&self) -> impl Iterator<Item = &BondInformation> {
        self.entries.iter().filter(|e| e.profile == self.profile).map(|e| &e.bond)
    }

    fn entry_mut(&mut self, peer: &Identity) -> Option<&mut Entry> {
        let profile = self.profile;
        self.entries.iter_mut().find(|e| e.profile == profile && e.bond.identity.match_identity(peer))
    }

    /// 今の送り先の同じ相手のものは置き換え、無ければ追加して保存する。ペアリングし直したので、購読の状態は消す。
    pub fn save(&mut self, bond: BondInformation) {
        let profile = self.profile;
        if let Some(e) = self.entries.iter_mut().find(|e| e.profile == profile && e.bond.identity.addr == bond.identity.addr) {
            e.bond = bond;
            e.cccd.clear();
        } else {
            if self.bonds().count() >= MAX_BONDS_PER_PROFILE {
                if let Some(i) = self.entries.iter().position(|e| e.profile == profile) {
                    self.entries.remove(i);
                }
            }
            self.entries.push(Entry { profile, bond, cccd: Vec::new() });
        }
        self.flush();
    }

    /// この起動でまだ戻していなければ、相手の保存済みの購読の状態を返す（以後は返さない）。
    pub fn cccd_to_restore(&mut self, peer: &Identity) -> Option<Vec<u8>> {
        if self.restored.iter().any(|r| r.match_identity(peer)) {
            return None;
        }
        let cccd = self.entry_mut(peer).map(|e| e.cccd.clone())?;
        self.restored.push(*peer);
        (!cccd.is_empty()).then_some(cccd)
    }

    /// 相手の今の購読の状態を、変わっていれば保存する（ボンド済みの相手だけ）。
    pub fn save_cccd(&mut self, peer: &Identity, cccd: &[u8]) {
        if cccd.len() > u8::MAX as usize {
            return;
        }
        let Some(e) = self.entry_mut(peer) else { return };
        if e.cccd == cccd {
            return;
        }
        e.cccd.clear();
        e.cccd.extend_from_slice(cccd);
        if !self.restored.iter().any(|r| r.match_identity(peer)) {
            self.restored.push(*peer);
        }
        info!("bond_store: subscriptions changed, saving");
        self.flush();
    }

    fn flush(&mut self) {
        let mut buf = [0xffu8; SECTOR];
        buf[..4].copy_from_slice(&MAGIC);
        let mut pos = 5;
        let mut count = 0u8;
        for e in &self.entries {
            let mut tmp = [0u8; 128];
            let Ok(enc) = postcard::to_slice(&e.bond, &mut tmp) else {
                warn!("bond_store: encode failed");
                return;
            };
            if pos + 3 + enc.len() + 1 + e.cccd.len() > SECTOR {
                warn!("bond_store: sector full, dropping the rest");
                break;
            }
            buf[pos] = e.profile;
            buf[pos + 1..pos + 3].copy_from_slice(&(enc.len() as u16).to_le_bytes());
            pos += 3;
            buf[pos..pos + enc.len()].copy_from_slice(enc);
            pos += enc.len();
            buf[pos] = e.cccd.len() as u8;
            pos += 1;
            buf[pos..pos + e.cccd.len()].copy_from_slice(&e.cccd);
            pos += e.cccd.len();
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
