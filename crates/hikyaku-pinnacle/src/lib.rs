//! Cirque Pinnacle（TM035035 などの円形トラックパッド）の SPI ドライバ。
//!
//! 通信は Cirque の RAP（Register Access Protocol）。コマンド形式は Cirque の
//! リファレンス実装（Arduino ライブラリ）に基づく:
//!
//! - 書き込み: `[0x80 | addr, value]`
//! - 読み出し: `[0xA0 | addr, 0xFC, 0xFC, 0xFC × n]` を送り、**4 バイト目以降**の n バイトがデータ
//!   （アドレスは自動インクリメント）
//! - SPI モード 1（CPOL=0, CPHA=1）、最大 13MHz
//!
//! 実機での確認は未了。コネクタが届いたら `Pinnacle::init` が返す ID/バージョンで疎通を確かめる。
//!
//! 絶対座標モードで使う想定（タップやスクロールはソフトウェア側で処理する）。

#![cfg_attr(not(test), no_std)]

use embedded_hal::spi::{Operation, SpiDevice};

/// レジスタアドレス。
pub mod reg {
    pub const FIRMWARE_ID: u8 = 0x00;
    pub const FIRMWARE_VERSION: u8 = 0x01;
    pub const STATUS1: u8 = 0x02;
    pub const SYS_CONFIG1: u8 = 0x03;
    pub const FEED_CONFIG1: u8 = 0x04;
    pub const FEED_CONFIG2: u8 = 0x05;
    pub const Z_IDLE: u8 = 0x0A;
    /// 絶対座標パケットの先頭（0x12〜0x17 の 6 バイト）。
    pub const PACKET_BYTE_0: u8 = 0x12;
}

/// Status1 のビット。
pub mod status {
    /// Software Data Ready（座標データが読める）。実機で確認: bit2。
    pub const SW_DR: u8 = 1 << 2;
    /// Software Command Complete（起動時の補正などの完了）。bit3。
    pub const SW_CC: u8 = 1 << 3;
}

const WRITE: u8 = 0x80;
const READ: u8 = 0xA0;
const FILLER: u8 = 0xFC;

/// FeedConfig1: Feed Enable
const FEED_ENABLE: u8 = 1 << 0;
/// FeedConfig1: 絶対座標モード
const FEED_ABSOLUTE: u8 = 1 << 1;
/// FeedConfig2: 全タップ・二次タップ・スクロール・GlideExtend を無効（ソフトウェアで処理する）
const FEED2_DISABLE_ALL_GESTURES: u8 = 0x1E;
/// 指を離した後に送る「空」パケットの数
const Z_IDLE_COUNT: u8 = 5;

/// 一度に読む最大バイト数。
pub const MAX_READ: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum Error<E> {
    Spi(E),
    /// 読み出しサイズが `MAX_READ` を超えた。
    TooLong,
}

/// 絶対座標の 1 サンプル。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbsoluteData {
    /// ボタン状態（BTN1〜3）。
    pub buttons: u8,
    /// 0〜2047（実用範囲は 128〜1920）。
    pub x: u16,
    /// 0〜1535（実用範囲は 64〜1472）。
    pub y: u16,
    /// 接触の強さ。0 なら指が触れていない。
    pub z: u8,
}

impl AbsoluteData {
    pub fn touching(&self) -> bool {
        self.z > 0
    }
}

/// 絶対座標パケット（レジスタ 0x12〜0x17）を解釈する。
///
/// X・Y とも 11 ビット（0〜2047）にマスクする。実機で、パケット 4 バイト目の bit7（Y の最上位に見えるビット）が
/// 座標ではない別のフラグとして立ったり立たなかったりすることを確認した（離したときは `x=0, y=2048` になる）。
/// マスクしないと Y が 2048 だけ跳ぶ。仕様上の範囲は X 0〜2048、Y 0〜1536 で、11 ビットに収まる。
pub fn parse_absolute(d: &[u8; 6]) -> AbsoluteData {
    AbsoluteData {
        buttons: d[0] & 0x3F,
        x: (d[2] as u16 | ((d[4] as u16 & 0x0F) << 8)) & 0x7FF,
        y: (d[3] as u16 | ((d[4] as u16 & 0xF0) << 4)) & 0x7FF,
        z: d[5] & 0x3F,
    }
}

/// `init` の結果。ファームウェア ID/バージョンで疎通を確認する（参考実装では ID=0x07, Ver=0x3A）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub firmware_id: u8,
    pub firmware_version: u8,
}

pub struct Pinnacle<SPI> {
    spi: SPI,
}

impl<SPI: SpiDevice> Pinnacle<SPI> {
    pub fn new(spi: SPI) -> Self {
        Self { spi }
    }

    pub fn release(self) -> SPI {
        self.spi
    }

    pub fn write_reg(&mut self, addr: u8, value: u8) -> Result<(), Error<SPI::Error>> {
        self.spi
            .write(&[WRITE | (addr & 0x1F), value])
            .map_err(Error::Spi)
    }

    pub fn read_regs(&mut self, addr: u8, out: &mut [u8]) -> Result<(), Error<SPI::Error>> {
        if out.len() > MAX_READ {
            return Err(Error::TooLong);
        }
        let n = 3 + out.len();
        let mut tx = [FILLER; 3 + MAX_READ];
        tx[0] = READ | (addr & 0x1F);
        let mut rx = [0u8; 3 + MAX_READ];
        self.spi
            .transaction(&mut [Operation::Transfer(&mut rx[..n], &tx[..n])])
            .map_err(Error::Spi)?;
        out.copy_from_slice(&rx[3..n]);
        Ok(())
    }

    pub fn read_reg(&mut self, addr: u8) -> Result<u8, Error<SPI::Error>> {
        let mut b = [0u8; 1];
        self.read_regs(addr, &mut b)?;
        Ok(b[0])
    }

    /// Status1 のフラグ（SW_DR / SW_CC）をクリアする。データを読んだ後に呼ぶ。
    pub fn clear_flags(&mut self) -> Result<(), Error<SPI::Error>> {
        self.write_reg(reg::STATUS1, 0x00)
    }

    /// 初期化して絶対座標モードで動かす。ID とバージョンを返す（検証は呼び出し側で）。
    pub fn init(&mut self) -> Result<Info, Error<SPI::Error>> {
        let firmware_id = self.read_reg(reg::FIRMWARE_ID)?;
        let firmware_version = self.read_reg(reg::FIRMWARE_VERSION)?;
        self.clear_flags()?;
        self.write_reg(reg::SYS_CONFIG1, 0x00)?; // シャットダウン/スリープを解除
        self.write_reg(reg::FEED_CONFIG2, FEED2_DISABLE_ALL_GESTURES)?;
        self.write_reg(reg::Z_IDLE, Z_IDLE_COUNT)?;
        self.write_reg(reg::FEED_CONFIG1, FEED_ENABLE | FEED_ABSOLUTE)?;
        // 設定中に立った SW_CC（コマンド完了）などをクリアする。クリアしないと DR ピンが High のまま張り付き、
        // Pinnacle は新しい座標を出さない。
        self.clear_flags()?;
        Ok(Info { firmware_id, firmware_version })
    }

    /// 新しいサンプルがあれば読んで返す。
    ///
    /// Status1 の SW_DR か SW_CC のどちらかが立っていればパケットを読み、フラグをクリアする
    /// （クリアしないと Pinnacle は新しい座標を出さず、DR ピンが High に張り付く）。
    /// それ以外のビットだけが立っているときは、クリアだけして `None`。
    /// DR ピンを見られるなら、ピンが立っているときだけ呼ぶと SPI の無駄打ちが減る。
    pub fn read_absolute(&mut self) -> Result<Option<AbsoluteData>, Error<SPI::Error>> {
        let st = self.read_reg(reg::STATUS1)?;
        if st & (status::SW_DR | status::SW_CC) == 0 {
            if st != 0 {
                self.clear_flags()?;
            }
            return Ok(None);
        }
        let mut d = [0u8; 6];
        self.read_regs(reg::PACKET_BYTE_0, &mut d)?;
        self.clear_flags()?;
        Ok(Some(parse_absolute(&d)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::convert::Infallible;
    use embedded_hal::spi::ErrorType;

    /// RAP を話すだけの簡易モック。レジスタ 32 個を持つ。
    struct Mock {
        regs: [u8; 32],
        /// 受信した書き込みの記録 (addr, value)
        writes: std::vec::Vec<(u8, u8)>,
    }

    impl Mock {
        fn new() -> Self {
            Self { regs: [0; 32], writes: std::vec::Vec::new() }
        }
    }

    impl ErrorType for Mock {
        type Error = Infallible;
    }

    impl SpiDevice for Mock {
        fn transaction(&mut self, ops: &mut [Operation<'_, u8>]) -> Result<(), Infallible> {
            for op in ops {
                match op {
                    Operation::Write(w) => {
                        assert_eq!(w.len(), 2);
                        assert_eq!(w[0] & 0xE0, 0x80, "write command");
                        let a = w[0] & 0x1F;
                        self.regs[a as usize] = w[1];
                        self.writes.push((a, w[1]));
                    }
                    Operation::Transfer(rx, tx) => {
                        assert_eq!(tx[0] & 0xE0, 0xA0, "read command");
                        assert!(tx[1..].iter().all(|&b| b == 0xFC), "filler bytes");
                        let a = (tx[0] & 0x1F) as usize;
                        rx[..3].fill(0xFB);
                        for i in 3..rx.len() {
                            rx[i] = self.regs[a + i - 3];
                        }
                    }
                    _ => panic!("unexpected op"),
                }
            }
            Ok(())
        }
    }

    #[test]
    fn parses_absolute_packet() {
        // x = 0x3A5 (低 0xA5 + 上位 3), y = 0x2C8 (低 0xC8 + 上位 2), z = 0x15
        let d = [0x01, 0x00, 0xA5, 0xC8, 0x23, 0x15];
        let a = parse_absolute(&d);
        assert_eq!(a.buttons, 1);
        assert_eq!(a.x, 0x3A5);
        assert_eq!(a.y, 0x2C8);
        assert_eq!(a.z, 0x15);
        assert!(a.touching());
    }

    #[test]
    fn masks_unused_bits() {
        let d = [0xFF, 0, 0, 0, 0, 0xFF];
        let a = parse_absolute(&d);
        assert_eq!(a.buttons, 0x3F);
        assert_eq!(a.z, 0x3F);
        let a = parse_absolute(&[0, 0, 0xFF, 0xFF, 0xFF, 0]);
        assert_eq!(a.x, 0x7FF);
        assert_eq!(a.y, 0x7FF);
        assert!(!a.touching());
    }

    #[test]
    fn y_bit11_is_ignored() {
        // 実機のログ: 離したときの (x=0, y=2048, z=0) は、パケット 4 バイト目の bit7 が立っただけ。
        let a = parse_absolute(&[0, 0, 0, 0, 0x80, 0]);
        assert_eq!((a.x, a.y, a.z), (0, 0, 0));
        // 触っている最中に bit7 が立っても立たなくても、同じ Y になる。
        let on = parse_absolute(&[0, 0, 0x00, 0x52, 0xA2, 0x0D]);
        let off = parse_absolute(&[0, 0, 0x00, 0x52, 0x22, 0x0D]);
        assert_eq!(on.y, off.y);
        assert_eq!(on.y, 0x252);
    }

    #[test]
    fn read_reg_skips_three_filler_bytes() {
        let mut m = Mock::new();
        m.regs[reg::FIRMWARE_ID as usize] = 0x07;
        m.regs[reg::FIRMWARE_VERSION as usize] = 0x3A;
        let mut p = Pinnacle::new(m);
        assert_eq!(p.read_reg(reg::FIRMWARE_ID).unwrap(), 0x07);
        let mut two = [0u8; 2];
        p.read_regs(reg::FIRMWARE_ID, &mut two).unwrap();
        assert_eq!(two, [0x07, 0x3A]);
    }

    #[test]
    fn rejects_oversized_read() {
        let mut p = Pinnacle::new(Mock::new());
        let mut big = [0u8; MAX_READ + 1];
        assert_eq!(p.read_regs(0, &mut big), Err(Error::TooLong));
    }

    #[test]
    fn init_configures_absolute_mode() {
        let mut m = Mock::new();
        m.regs[0] = 0x07;
        m.regs[1] = 0x3A;
        let mut p = Pinnacle::new(m);
        let info = p.init().unwrap();
        assert_eq!(info, Info { firmware_id: 0x07, firmware_version: 0x3A });
        let m = p.release();
        assert_eq!(m.regs[reg::FEED_CONFIG1 as usize], 0x03);
        assert_eq!(m.regs[reg::FEED_CONFIG2 as usize], 0x1E);
        assert_eq!(m.regs[reg::Z_IDLE as usize], 5);
        assert_eq!(m.regs[reg::SYS_CONFIG1 as usize], 0x00);
        // Feed を有効にした後、最後にフラグをクリアする（SW_CC が残ると Pinnacle が止まる）
        assert_eq!(m.writes.last(), Some(&(reg::STATUS1, 0x00)));
        assert_eq!(m.writes[m.writes.len() - 2], (reg::FEED_CONFIG1, 0x03));
    }

    #[test]
    fn read_absolute_returns_none_without_data_ready() {
        let mut p = Pinnacle::new(Mock::new());
        assert_eq!(p.read_absolute().unwrap(), None);
    }

    #[test]
    fn read_absolute_reads_and_clears_on_command_complete_too() {
        // SW_CC だけが立っていても、読んでクリアする（ビットの割り当てを取り違えていても止まらないように）。
        let mut m = Mock::new();
        m.regs[reg::STATUS1 as usize] = status::SW_CC;
        let mut p = Pinnacle::new(m);
        let a = p.read_absolute().unwrap().expect("packet");
        assert!(!a.touching());
        assert_eq!(p.release().regs[reg::STATUS1 as usize], 0, "flag cleared");
    }

    #[test]
    fn read_absolute_clears_unknown_flags_without_data() {
        let mut m = Mock::new();
        m.regs[reg::STATUS1 as usize] = 0x01;
        let mut p = Pinnacle::new(m);
        assert_eq!(p.read_absolute().unwrap(), None);
        assert_eq!(p.release().regs[reg::STATUS1 as usize], 0);
    }

    #[test]
    fn status_bits_match_hardware() {
        assert_eq!(status::SW_DR, 0x04);
        assert_eq!(status::SW_CC, 0x08);
    }

    #[test]
    fn read_absolute_reads_packet_and_clears_flags() {
        let mut m = Mock::new();
        m.regs[reg::STATUS1 as usize] = status::SW_DR;
        m.regs[0x12..0x18].copy_from_slice(&[0, 0, 0x80, 0x40, 0x10, 0x20]);
        let mut p = Pinnacle::new(m);
        let a = p.read_absolute().unwrap().unwrap();
        assert_eq!((a.x, a.y, a.z), (0x80, 0x140, 0x20));
        let m = p.release();
        assert_eq!(m.regs[reg::STATUS1 as usize], 0, "flags cleared");
    }
}
