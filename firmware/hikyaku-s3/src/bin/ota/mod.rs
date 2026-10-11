//! BLE 経由のファーム更新（OTA）。
//!
//! 流れ（PC 側は tools/ota/upload.py）:
//!   1. キーボードの特定の組み合わせで「OTA モード」に入る（ホストとの接続を切って `hikyaku-OTA` として待つ）
//!   2. PC が接続し、HELLO → ナンス受信 → START（サイズ・SHA-256・HMAC）→ データ送信 → FINISH
//!   3. 検証に通ったら次のスロットを有効化して再起動
//!
//! 守り: 物理操作（キー入力）で入る OTA モードだけが受け付ける + 時間制限 + 共有鍵による HMAC 認証
//! + SHA-256 での整合性確認。鍵はビルド時の環境変数 `HIKYAKU_OTA_KEY`（64 桁の 16 進）。
//! ロールバックは無いが、BOOT ボタン + 有線書き込みでいつでも復旧できる。
//!
//! プロトコル（ctrl キャラクタリスティックへの書き込み / 通知）:
//!   書き込み: `01` HELLO | `02 size:u32le sha256[32] hmac[32]` START | `03` FINISH | `04` ABORT
//!   通知:     `81 nonce[16]` | `82 status` | `83 status` | `90 offset:u32le`（進捗。フロー制御用）
//! データは data キャラクタリスティックへ 240 バイト単位で write-without-response。
//! HMAC = HMAC-SHA256(key, nonce ‖ size:u32le ‖ sha256)。

use alloc::vec::Vec;
use core::cell::RefCell;

use esp_bootloader_esp_idf::{
    ota::OtaImageState,
    ota_updater::OtaUpdater,
    partitions::{FlashRegion, PARTITION_TABLE_MAX_LEN},
};
use esp_hal::rng::Rng;
use esp_storage::FlashStorage;
use log::{info, warn};
use sha2::{Digest, Sha256};

/// ctrl キャラクタリスティックの値の長さ（通知は常にこの長さ）。
pub const CTRL_LEN: usize = 80;
/// data キャラクタリスティックの値の長さ。
pub const DATA_LEN: usize = 244;

const SECTOR: u32 = 4096;
/// この量を受け取るたびに進捗を通知する（ホストはそれを待って次を送る）。
const ACK_EVERY: u32 = 16 * 1024;

mod op {
    pub const HELLO: u8 = 0x01;
    pub const START: u8 = 0x02;
    pub const FINISH: u8 = 0x03;
    pub const ABORT: u8 = 0x04;
    pub const R_NONCE: u8 = 0x81;
    pub const R_START: u8 = 0x82;
    pub const R_FINISH: u8 = 0x83;
    pub const R_PROGRESS: u8 = 0x90;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    Ok = 0,
    BadState = 1,
    AuthFailed = 2,
    TooBig = 3,
    Flash = 4,
    HashMismatch = 5,
    /// パーティションテーブルに OTA 用のスロットが無い（有線で書き直す必要がある）
    NotOta = 6,
    BadImage = 7,
    BadRequest = 8,
}

/// ビルド時の鍵（`HIKYAKU_OTA_KEY`）。無ければ OTA は無効。
pub fn key() -> Option<[u8; 32]> {
    let hex = option_env!("HIKYAKU_OTA_KEY")?.as_bytes();
    if hex.len() != 64 {
        return None;
    }
    let nib = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = nib(hex[2 * i])? << 4 | nib(hex[2 * i + 1])?;
    }
    Some(out)
}

fn hmac_sha256(key: &[u8; 32], parts: &[&[u8]]) -> [u8; 32] {
    let mut k = [0u8; 64];
    k[..32].copy_from_slice(key);
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    for p in parts {
        inner.update(p);
    }
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    outer.finalize().into()
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// ナンスを渡した。START を待つ。
    Hello,
    Receiving,
}

/// ctrl への書き込みに対する返答。
pub enum Reply {
    None,
    Notify([u8; CTRL_LEN]),
    /// 通知を送った後、再起動する（FINISH 成功時）。
    NotifyThenReboot([u8; CTRL_LEN]),
}

fn note(op: u8, body: &[u8]) -> [u8; CTRL_LEN] {
    let mut v = [0u8; CTRL_LEN];
    v[0] = op;
    v[1..1 + body.len()].copy_from_slice(body);
    v
}

pub struct Ota<'a, 'd> {
    flash: &'a RefCell<FlashStorage<'d>>,
    key: [u8; 32],
    state: State,
    nonce: [u8; 16],
    size: u32,
    expected: [u8; 32],
    hasher: Sha256,
    written: u32,
    erased: u32,
    acked: u32,
    /// パーティションテーブルの読み出し用（スタックに置かないようヒープに持つ）。
    pt_buf: Vec<u8>,
}

impl<'a, 'd> Ota<'a, 'd> {
    pub fn new(flash: &'a RefCell<FlashStorage<'d>>, key: [u8; 32]) -> Self {
        Self {
            flash,
            key,
            state: State::Idle,
            nonce: [0; 16],
            size: 0,
            expected: [0; 32],
            hasher: Sha256::new(),
            written: 0,
            erased: 0,
            acked: 0,
            pt_buf: alloc::vec![0u8; PARTITION_TABLE_MAX_LEN],
        }
    }

    /// 次に書き込むスロットのフラッシュ領域に対して `f` を実行する。
    fn with_region<R>(
        &mut self,
        f: impl FnOnce(&mut FlashRegion<'_, '_>) -> Result<R, esp_bootloader_esp_idf::partitions::Error>,
    ) -> Result<R, Status> {
        let mut flash = self.flash.borrow_mut();
        let buf: &mut [u8; PARTITION_TABLE_MAX_LEN] = (&mut self.pt_buf[..]).try_into().map_err(|_| Status::Flash)?;
        let mut up = OtaUpdater::new(&mut flash, buf).map_err(|_| Status::NotOta)?;
        let (mut region, _slot) = up.next_partition().map_err(|_| Status::NotOta)?;
        f(&mut region).map_err(|_| Status::Flash)
    }

    fn reset(&mut self) {
        self.state = State::Idle;
        self.hasher = Sha256::new();
        self.written = 0;
        self.erased = 0;
        self.acked = 0;
    }

    /// ctrl への書き込みを処理する。
    pub fn ctrl(&mut self, msg: &[u8]) -> Reply {
        match msg.first().copied() {
            Some(op::HELLO) => {
                self.reset();
                let rng = Rng::new();
                for c in self.nonce.chunks_mut(4) {
                    c.copy_from_slice(&rng.random().to_le_bytes());
                }
                self.state = State::Hello;
                Reply::Notify(note(op::R_NONCE, &self.nonce))
            }
            Some(op::START) => Reply::Notify(note(op::R_START, &[self.start(msg) as u8])),
            Some(op::FINISH) => match self.finish() {
                Status::Ok => Reply::NotifyThenReboot(note(op::R_FINISH, &[Status::Ok as u8])),
                st => {
                    self.reset();
                    Reply::Notify(note(op::R_FINISH, &[st as u8]))
                }
            },
            Some(op::ABORT) => {
                self.reset();
                Reply::None
            }
            _ => Reply::None,
        }
    }

    fn start(&mut self, msg: &[u8]) -> Status {
        if self.state != State::Hello {
            return Status::BadState;
        }
        // 認証の成否にかかわらず、ナンスは 1 回限り。
        self.state = State::Idle;
        if msg.len() != 1 + 4 + 32 + 32 {
            return Status::BadRequest;
        }
        let size_bytes = &msg[1..5];
        let sha = &msg[5..37];
        let mac = &msg[37..69];
        let want = hmac_sha256(&self.key, &[&self.nonce, size_bytes, sha]);
        if !ct_eq(&want, mac) {
            warn!("ota: authentication failed");
            return Status::AuthFailed;
        }
        let size = u32::from_le_bytes([size_bytes[0], size_bytes[1], size_bytes[2], size_bytes[3]]);
        let capacity = match self.with_region(|r| Ok(r.partition_size() as u32)) {
            Ok(c) => c,
            Err(e) => return e,
        };
        if size == 0 || size > capacity {
            return Status::TooBig;
        }
        self.size = size;
        self.expected.copy_from_slice(sha);
        self.hasher = Sha256::new();
        self.written = 0;
        self.erased = 0;
        self.acked = 0;
        self.state = State::Receiving;
        info!("ota: start, {} bytes (slot capacity {})", size, capacity);
        Status::Ok
    }

    /// data への書き込みを処理する。進捗を通知すべきときは `Ok(Some(offset))`。
    pub fn data(&mut self, chunk: &[u8]) -> Result<Option<u32>, Status> {
        if self.state != State::Receiving {
            return Err(Status::BadState);
        }
        let len = chunk.len() as u32;
        if len == 0 || len as usize > DATA_LEN || self.written + len > self.size {
            self.reset();
            return Err(Status::TooBig);
        }
        self.hasher.update(chunk);
        // フラッシュは 4 バイト単位で書く。末尾は 0xFF で埋める。
        let mut padded = [0xFFu8; DATA_LEN + 4];
        padded[..chunk.len()].copy_from_slice(chunk);
        let plen = (chunk.len() + 3) & !3;
        let off = self.written;
        let need = (off + plen as u32 + SECTOR - 1) / SECTOR * SECTOR;
        let erased = self.erased;
        let res = self.with_region(|r| {
            if need > erased {
                r.erase(erased, need)?;
            }
            r.write(off, &padded[..plen])
        });
        if let Err(e) = res {
            self.reset();
            return Err(e);
        }
        self.erased = self.erased.max(need);
        self.written += len;
        if self.written - self.acked >= ACK_EVERY {
            self.acked = self.written;
            return Ok(Some(self.written));
        }
        Ok(None)
    }

    /// データ受信中のエラーを通知する（FINISH の応答と同じ形式）。
    pub fn error_note(st: Status) -> [u8; CTRL_LEN] {
        note(op::R_FINISH, &[st as u8])
    }

    pub fn progress_note(offset: u32) -> [u8; CTRL_LEN] {
        note(op::R_PROGRESS, &offset.to_le_bytes())
    }

    fn finish(&mut self) -> Status {
        if self.state != State::Receiving || self.written != self.size {
            return Status::BadState;
        }
        let digest: [u8; 32] = core::mem::replace(&mut self.hasher, Sha256::new()).finalize().into();
        if !ct_eq(&digest, &self.expected) {
            warn!("ota: hash mismatch");
            return Status::HashMismatch;
        }
        // ESP イメージの先頭バイトは 0xE9。
        let mut first = [0u8; 4];
        match self.with_region(|r| {
            r.read(0, &mut first)?;
            Ok(())
        }) {
            Ok(()) if first[0] == 0xE9 => {}
            Ok(()) => return Status::BadImage,
            Err(e) => return e,
        }
        let mut flash = self.flash.borrow_mut();
        let buf: &mut [u8; PARTITION_TABLE_MAX_LEN] = match (&mut self.pt_buf[..]).try_into() {
            Ok(b) => b,
            Err(_) => return Status::Flash,
        };
        let res = OtaUpdater::new(&mut flash, buf).and_then(|mut up| up.activate_next_partition());
        match res {
            Ok(()) => {
                info!("ota: image verified, next slot activated");
                Status::Ok
            }
            Err(_) => Status::Flash,
        }
    }
}

/// 起動できたことを bootloader に伝える（OTA 直後の画像を「有効」にする）。OTA 用のパーティションが無ければ何もしない。
pub fn mark_app_valid(flash: &RefCell<FlashStorage<'_>>) {
    let mut buf = alloc::vec![0u8; PARTITION_TABLE_MAX_LEN];
    let Ok(buf) = <&mut [u8; PARTITION_TABLE_MAX_LEN]>::try_from(&mut buf[..]) else { return };
    let mut flash = flash.borrow_mut();
    let Ok(mut up) = OtaUpdater::new(&mut flash, buf) else {
        info!("ota: no OTA partitions (flashed with the old layout)");
        return;
    };
    match up.current_ota_state() {
        Ok(OtaImageState::Valid) => {}
        Ok(state) => {
            if up.set_current_ota_state(OtaImageState::Valid).is_ok() {
                info!("ota: marked the running image valid (was {:?})", state);
            }
        }
        Err(_) => {}
    }
}
