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
//! 相対モード（パッド自身が計算した移動量）だけを扱う。絶対座標モードの経路は 2026-10-13 に削除した
//! （タグ `absolute-mode-last` に残っている）。

#![cfg_attr(not(test), no_std)]

use embedded_hal::{
    delay::DelayNs,
    spi::{Operation, SpiDevice},
};

/// レジスタアドレス。
pub mod reg {
    pub const FIRMWARE_ID: u8 = 0x00;
    pub const FIRMWARE_VERSION: u8 = 0x01;
    pub const STATUS1: u8 = 0x02;
    pub const SYS_CONFIG1: u8 = 0x03;
    pub const FEED_CONFIG1: u8 = 0x04;
    pub const FEED_CONFIG2: u8 = 0x05;
    pub const CAL_CONFIG1: u8 = 0x07;
    pub const Z_IDLE: u8 = 0x0A;
    /// ERA（拡張レジスタ）アクセス用: 値 / アドレス上位 / アドレス下位 / 制御
    pub const ERA_VALUE: u8 = 0x1B;
    pub const ERA_ADDR_HIGH: u8 = 0x1C;
    pub const ERA_ADDR_LOW: u8 = 0x1D;
    pub const ERA_CTRL: u8 = 0x1E;
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

/// SysConfig1: リセット
const SYS_RESET: u8 = 0x01;
/// CalConfig1: キャリブレーション開始（完了するとビットが落ちる）
const CAL_START: u8 = 0x01;
/// ERA 制御
const ERA_READ: u8 = 0x01;
const ERA_WRITE: u8 = 0x02;
const ERA_INC_READ: u8 = 0x04;
/// ERA: ADC 設定（上位 2 ビットが ADC の減衰）
const ERA_ADC_CONFIG: u16 = 0x0187;
const ADC_ATTENUATE_MASK: u8 = 0xC0;

/// ADC の減衰（オーバーレイの厚みに合わせる）。QMK の既定は、平らなオーバーレイで 4X、曲面で 2X。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attenuation {
    X1 = 0x00,
    X2 = 0x40,
    X3 = 0x80,
    X4 = 0xC0,
}

/// FeedConfig2（相対モード用）
const FEED2_ALL_TAP_DISABLE: u8 = 0x02;
const FEED2_SECONDARY_TAP_DISABLE: u8 = 0x04;
const FEED2_SCROLL_DISABLE: u8 = 0x08;
const FEED2_GLIDE_EXTEND_DISABLE: u8 = 0x10;
const FEED2_SWAP_XY: u8 = 0x80;
/// FeedConfig1: 反転（データシートでは絶対座標用だが、相対モードでも効く。Zephyr のコメントより）
const FEED1_X_INVERT: u8 = 0x40;
const FEED1_Y_INVERT: u8 = 0x80;

/// FeedConfig1: Feed Enable
const FEED_ENABLE: u8 = 1 << 0;
/// FeedConfig1: 絶対座標モード（0 なら相対モード）
const FEED_ABSOLUTE: u8 = 1 << 1;

/// 一度に読む最大バイト数。
pub const MAX_READ: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum Error<E> {
    Spi(E),
    /// 読み出しサイズが `MAX_READ` を超えた。
    TooLong,
    /// ファームウェア ID が Pinnacle（0x07）ではない。配線（特に MISO・電源）が外れていることが多い。
    BadId(u8),
}

/// Pinnacle（1CA027）のファームウェア ID。
pub const FIRMWARE_ID: u8 = 0x07;

/// 絶対座標の 1 サンプル（指の位置と接触の強さ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Absolute {
    /// 0〜2047（実用範囲はおよそ 128〜1920）。
    pub x: u16,
    /// 0〜2047（実用範囲はおよそ 64〜1472）。
    pub y: u16,
    /// 接触の強さ。0 なら指が触れていない（離した直後は ZIdle の数だけ z=0 のパケットが来る）。
    pub z: u8,
}

/// 絶対座標パケット（レジスタ 0x12〜0x17）を解釈する。
///
/// X・Y とも 11 ビットにマスクする。実機（2026-10-12）で、4 バイト目の bit7（Y の最上位に見えるビット）が座標ではない
/// 別のフラグとして立ったり立たなかったりした（マスクしないと Y が 2048 跳ぶ）。仕様上の範囲は 11 ビットに収まる。
pub fn parse_absolute(d: &[u8; 6]) -> Absolute {
    Absolute {
        x: (d[2] as u16 | ((d[4] as u16 & 0x0F) << 8)) & 0x7FF,
        y: (d[3] as u16 | ((d[4] as u16 & 0xF0) << 4)) & 0x7FF,
        z: d[5] & 0x3F,
    }
}

/// 相対モードの 1 サンプル（パッド自身が計算した移動量）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relative {
    /// bit0〜2: ボタン（タップを有効にしている場合は、タップによるクリックもここに出る）
    pub buttons: u8,
    pub dx: i16,
    /// Y の向きは `RelativeOptions::invert_y` で決まる（パッド側で反転する）。
    pub dy: i16,
    /// デコード前のバイト（レジスタ 0x12〜0x14）。診断用。
    pub raw: [u8; 3],
}

/// 相対モードのパケット（レジスタ 0x12〜0x14）を解釈する（Zephyr / ZMK の `pinnacle_decode_sample` と同じ）。
/// 1 バイト目の bit4/bit5 が X/Y の符号（9 ビットの 2 の補数）。Y の向きはパッド側の設定（FeedConfig1 の反転ビット）で決める。
pub fn parse_relative(d: &[u8; 3]) -> Relative {
    let dx = if d[0] & 0x10 != 0 { d[1] as i16 - 256 } else { d[1] as i16 };
    let dy = if d[0] & 0x20 != 0 { d[2] as i16 - 256 } else { d[2] as i16 };
    Relative { buttons: d[0] & 0x07, dx, dy, raw: *d }
}

/// 相対モードの設定（Zephyr の devicetree の `cirque,pinnacle` と同じ項目）。
#[derive(Clone, Copy, Debug)]
pub struct RelativeOptions {
    pub sensitivity: Attenuation,
    /// パッドのタップでクリック（`Relative::buttons` の bit0）。
    pub primary_tap: bool,
    pub invert_x: bool,
    pub invert_y: bool,
    pub swap_xy: bool,
    /// ZIdle: 指を離したときにパッドが送る「空」パケット（相対モードでは dx=dy=0）の数。
    /// 0（ZMK と同じ）なら送らない。QMK は 5。ソフト側でタップを判定するときの「離した」合図に使える。
    pub z_idle: u8,
}

impl Default for RelativeOptions {
    /// タップ有効、感度 4X、反転なし、ZIdle 0。
    /// パッド側の反転（FeedConfig1 の bit6/bit7）はデータシートでは絶対座標用。実機では相対モードで Y 反転を入れると
    /// Y の移動量がほとんど出なくなったので、既定では使わない（向きは受け取った側で合わせる）。
    fn default() -> Self {
        Self { sensitivity: Attenuation::X4, primary_tap: true, invert_x: false, invert_y: false, swap_xy: false, z_idle: 0 }
    }
}

/// `init_relative` の結果。ファームウェア ID/バージョンで疎通を確認する（参考実装では ID=0x07, Ver=0x3A）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub firmware_id: u8,
    pub firmware_version: u8,
}

pub struct Pinnacle<SPI> {
    spi: SPI,
    /// CS を落としてから最初のクロックまでの待ち（ns）。0 なら待たない。
    cs_setup_ns: u32,
}

impl<SPI: SpiDevice> Pinnacle<SPI> {
    pub fn new(spi: SPI) -> Self {
        Self { spi, cs_setup_ns: 0 }
    }

    /// CS を落としてから最初のクロックまでの待ち（ns）を設定する（SPI の物理層の調整用）。
    pub fn set_cs_setup_ns(&mut self, ns: u32) {
        self.cs_setup_ns = ns;
    }

    pub fn release(self) -> SPI {
        self.spi
    }

    /// SPI の設定（周波数など）を変えるために、下のデバイスを借りる。
    pub fn spi_mut(&mut self) -> &mut SPI {
        &mut self.spi
    }

    pub fn write_reg(&mut self, addr: u8, value: u8) -> Result<(), Error<SPI::Error>> {
        let buf = [WRITE | (addr & 0x1F), value];
        if self.cs_setup_ns > 0 {
            self.spi
                .transaction(&mut [Operation::DelayNs(self.cs_setup_ns), Operation::Write(&buf)])
                .map_err(Error::Spi)
        } else {
            self.spi.write(&buf).map_err(Error::Spi)
        }
    }

    pub fn read_regs(&mut self, addr: u8, out: &mut [u8]) -> Result<(), Error<SPI::Error>> {
        if out.len() > MAX_READ {
            return Err(Error::TooLong);
        }
        let n = 3 + out.len();
        let mut tx = [FILLER; 3 + MAX_READ];
        tx[0] = READ | (addr & 0x1F);
        let mut rx = [0u8; 3 + MAX_READ];
        if self.cs_setup_ns > 0 {
            self.spi
                .transaction(&mut [Operation::DelayNs(self.cs_setup_ns), Operation::Transfer(&mut rx[..n], &tx[..n])])
                .map_err(Error::Spi)?;
        } else {
            self.spi
                .transaction(&mut [Operation::Transfer(&mut rx[..n], &tx[..n])])
                .map_err(Error::Spi)?;
        }
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

    /// 相対モードで初期化する（Zephyr / ZMK の `pinnacle_init` と同じ手順）。
    ///
    /// ソフトリセット → 電源投入時のキャリブレーション完了（SW_CC）を待つ → 感度（ADC 減衰）→ キャリブレーション →
    /// FeedConfig2（GlideExtend・スクロール・二次タップ無効。IntelliMouse は使わない＝3 バイトのパケット）→
    /// FeedConfig1（相対モードで有効化 + 反転）→ ZIdle（既定 0）。
    /// **起動中（約 0.5 秒）はパッドに触れないこと**（補正が狂う）。
    pub fn init_relative<D: DelayNs>(&mut self, delay: &mut D, opt: RelativeOptions) -> Result<Info, Error<SPI::Error>> {
        let firmware_id = self.read_reg(reg::FIRMWARE_ID)?;
        let firmware_version = self.read_reg(reg::FIRMWARE_VERSION)?;
        // Zephyr と同じく、ID が違えば初期化しない（配線が外れていると 0x00 や 0xFC が読め、ゴミを移動量として送ってしまう）。
        if firmware_id != FIRMWARE_ID {
            return Err(Error::BadId(firmware_id));
        }

        self.clear_flags_wait(delay)?;
        self.write_reg(reg::SYS_CONFIG1, SYS_RESET)?;
        // 電源投入時のキャリブレーション完了（SW_CC）を待つ（最大 200ms）
        for _ in 0..200 {
            delay.delay_ms(1);
            if self.read_reg(reg::STATUS1)? & status::SW_CC != 0 {
                break;
            }
        }
        self.clear_flags_wait(delay)?;

        if self.set_adc_attenuation(delay, opt.sensitivity)? {
            self.calibrate(delay)?;
        }

        self.write_reg(reg::SYS_CONFIG1, 0x00)?;
        let mut f2 = FEED2_GLIDE_EXTEND_DISABLE | FEED2_SCROLL_DISABLE | FEED2_SECONDARY_TAP_DISABLE;
        if opt.swap_xy {
            f2 |= FEED2_SWAP_XY;
        }
        if !opt.primary_tap {
            f2 |= FEED2_ALL_TAP_DISABLE;
        }
        self.write_reg(reg::FEED_CONFIG2, f2)?;

        let mut f1 = FEED_ENABLE; // 相対モード（データ形式ビットなし）
        if opt.invert_x {
            f1 |= FEED1_X_INVERT;
        }
        if opt.invert_y {
            f1 |= FEED1_Y_INVERT;
        }
        self.write_reg(reg::FEED_CONFIG1, f1)?;
        self.write_reg(reg::Z_IDLE, opt.z_idle)?;
        self.clear_flags_wait(delay)?;
        Ok(Info { firmware_id, firmware_version })
    }

    /// 相対モードで、新しいサンプルがあれば読んで返す。
    pub fn read_relative(&mut self) -> Result<Option<Relative>, Error<SPI::Error>> {
        let st = self.read_reg(reg::STATUS1)?;
        if st & (status::SW_DR | status::SW_CC) == 0 {
            if st != 0 {
                self.clear_flags()?;
            }
            return Ok(None);
        }
        let mut d = [0u8; 3];
        self.read_regs(reg::PACKET_BYTE_0, &mut d)?;
        self.clear_flags()?;
        if st & status::SW_DR == 0 {
            return Ok(None); // データではなく、コマンド完了だけ
        }
        Ok(Some(parse_relative(&d)))
    }

    /// 相対モードと絶対座標モードを切り替える（FeedConfig1 の bit1 だけを変え、ほかの設定は残す）。
    /// `z_idle` は、指を離したときに送る「空」パケット（z=0）の数。絶対座標モードでは、離したことを知るのに使う。
    /// キャリブレーションはやり直さない。
    pub fn set_absolute<D: DelayNs>(&mut self, delay: &mut D, absolute: bool, z_idle: u8) -> Result<(), Error<SPI::Error>> {
        let f1 = self.read_reg(reg::FEED_CONFIG1)?;
        let f1 = if absolute { f1 | FEED_ABSOLUTE } else { f1 & !FEED_ABSOLUTE };
        self.write_reg(reg::FEED_CONFIG1, f1 | FEED_ENABLE)?;
        self.write_reg(reg::Z_IDLE, z_idle)?;
        self.clear_flags_wait(delay)
    }

    /// 絶対座標モードで、新しいサンプルがあれば読んで返す。
    pub fn read_absolute(&mut self) -> Result<Option<Absolute>, Error<SPI::Error>> {
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
        if st & status::SW_DR == 0 {
            return Ok(None);
        }
        Ok(Some(parse_absolute(&d)))
    }

    /// フラグをクリアして、50µs 待つ（QMK と同じ）。
    pub fn clear_flags_wait<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<SPI::Error>> {
        self.clear_flags()?;
        delay.delay_us(50);
        Ok(())
    }

    pub fn enable_feed(&mut self, enable: bool) -> Result<(), Error<SPI::Error>> {
        let mut v = self.read_reg(reg::FEED_CONFIG1)?;
        if enable {
            v |= FEED_ENABLE;
        } else {
            v &= !FEED_ENABLE;
        }
        self.write_reg(reg::FEED_CONFIG1, v)
    }

    /// 拡張レジスタ（ERA）を読む。Feed を止めてから行う。
    pub fn era_read<D: DelayNs>(&mut self, delay: &mut D, addr: u16) -> Result<u8, Error<SPI::Error>> {
        self.enable_feed(false)?;
        self.write_reg(reg::ERA_ADDR_HIGH, (addr >> 8) as u8)?;
        self.write_reg(reg::ERA_ADDR_LOW, addr as u8)?;
        self.write_reg(reg::ERA_CTRL, ERA_INC_READ | ERA_READ)?;
        self.wait_era_done(delay)?;
        let v = self.read_reg(reg::ERA_VALUE)?;
        self.clear_flags_wait(delay)?;
        Ok(v)
    }

    /// 拡張レジスタ（ERA）に書く。
    pub fn era_write<D: DelayNs>(&mut self, delay: &mut D, addr: u16, value: u8) -> Result<(), Error<SPI::Error>> {
        self.enable_feed(false)?;
        self.write_reg(reg::ERA_VALUE, value)?;
        self.write_reg(reg::ERA_ADDR_HIGH, (addr >> 8) as u8)?;
        self.write_reg(reg::ERA_ADDR_LOW, addr as u8)?;
        self.write_reg(reg::ERA_CTRL, ERA_WRITE)?;
        self.wait_era_done(delay)?;
        self.clear_flags_wait(delay)
    }

    /// ERA 制御レジスタが 0 に戻るのを待つ（最大 20ms）。
    fn wait_era_done<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<SPI::Error>> {
        for _ in 0..200 {
            if self.read_reg(reg::ERA_CTRL)? == 0 {
                return Ok(());
            }
            delay.delay_us(100);
        }
        Ok(())
    }

    /// ADC の減衰を設定する。変更した（キャリブレーションが必要）なら true。
    pub fn set_adc_attenuation<D: DelayNs>(&mut self, delay: &mut D, a: Attenuation) -> Result<bool, Error<SPI::Error>> {
        let cur = self.era_read(delay, ERA_ADC_CONFIG)?;
        if cur & ADC_ATTENUATE_MASK == a as u8 {
            return Ok(false);
        }
        self.era_write(delay, ERA_ADC_CONFIG, (cur & !ADC_ATTENUATE_MASK) | a as u8)?;
        Ok(true)
    }

    /// キャリブレーションする（約 100ms。最大 200ms 待つ）。
    pub fn calibrate<D: DelayNs>(&mut self, delay: &mut D) -> Result<(), Error<SPI::Error>> {
        let v = self.read_reg(reg::CAL_CONFIG1)?;
        self.write_reg(reg::CAL_CONFIG1, v | CAL_START)?;
        for _ in 0..200 {
            if self.read_reg(reg::CAL_CONFIG1)? & CAL_START == 0 {
                break;
            }
            delay.delay_ms(1);
        }
        self.clear_flags_wait(delay)
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
        /// 拡張レジスタ（ERA）の中身（アドレス 0x0000〜0x01FF）
        era: [u8; 0x200],
        calibrations: u32,
    }

    impl Mock {
        fn new() -> Self {
            let mut era = [0u8; 0x200];
            era[0x0187] = 0x4E; // ADC 設定の既定値（減衰 2X）
            Self { regs: [0; 32], writes: std::vec::Vec::new(), era, calibrations: 0 }
        }

        /// 書き込みに対するパッド側の動作（ERA の読み書き、キャリブレーション完了）。
        fn on_write(&mut self, addr: u8, value: u8) {
            if addr == reg::ERA_CTRL {
                let a = ((self.regs[reg::ERA_ADDR_HIGH as usize] as usize) << 8) | self.regs[reg::ERA_ADDR_LOW as usize] as usize;
                if value & ERA_WRITE != 0 {
                    self.era[a] = self.regs[reg::ERA_VALUE as usize];
                }
                if value & ERA_READ != 0 {
                    self.regs[reg::ERA_VALUE as usize] = self.era[a];
                }
                self.regs[reg::ERA_CTRL as usize] = 0; // 完了
            }
            if addr == reg::CAL_CONFIG1 && value & CAL_START != 0 {
                self.regs[reg::CAL_CONFIG1 as usize] &= !CAL_START; // すぐ完了
                self.calibrations += 1;
            }
        }
    }

    struct NoDelay;
    impl DelayNs for NoDelay {
        fn delay_ns(&mut self, _ns: u32) {}
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
                        self.on_write(a, w[1]);
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
                    Operation::DelayNs(_) => {}
                    _ => panic!("unexpected op"),
                }
            }
            Ok(())
        }
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
    fn era_write_and_read_round_trip() {
        let mut p = Pinnacle::new(Mock::new());
        p.era_write(&mut NoDelay, 0x0149, 0x04).unwrap();
        assert_eq!(p.era_read(&mut NoDelay, 0x0149).unwrap(), 0x04);
        assert_eq!(p.era_read(&mut NoDelay, 0x0187).unwrap(), 0x4E);
    }

    #[test]
    fn read_relative_returns_none_without_data_ready() {
        let mut p = Pinnacle::new(Mock::new());
        assert_eq!(p.read_relative().unwrap(), None);
    }

    #[test]
    fn read_relative_clears_command_complete_without_returning_data() {
        // SW_CC だけが立っていたら、読んでクリアする（止まらないように）が、データとしては返さない。
        let mut m = Mock::new();
        m.regs[reg::STATUS1 as usize] = status::SW_CC;
        let mut p = Pinnacle::new(m);
        assert_eq!(p.read_relative().unwrap(), None);
        assert_eq!(p.release().regs[reg::STATUS1 as usize], 0, "flag cleared");
    }

    #[test]
    fn read_relative_clears_unknown_flags_without_data() {
        let mut m = Mock::new();
        m.regs[reg::STATUS1 as usize] = 0x01;
        let mut p = Pinnacle::new(m);
        assert_eq!(p.read_relative().unwrap(), None);
        assert_eq!(p.release().regs[reg::STATUS1 as usize], 0);
    }

    #[test]
    fn parses_relative_packet_like_zephyr() {
        let r = parse_relative(&[0x00, 5, 3]);
        assert_eq!((r.buttons, r.dx, r.dy), (0, 5, 3));
        // 実機のログ: 符号ビット付きの 0xF8 → -8
        let r = parse_relative(&[0x18 | 0x20, 0xF8, 0xFE]);
        assert_eq!((r.buttons, r.dx, r.dy), (0, -8, -2));
        assert_eq!(parse_relative(&[0x09, 0, 0]).buttons, 1);
        // 9 ビットの範囲
        assert_eq!(parse_relative(&[0x10, 0x00, 0]).dx, -256);
        assert_eq!(parse_relative(&[0x00, 0xFF, 0]).dx, 255);
    }

    #[test]
    fn init_relative_follows_the_zephyr_sequence() {
        let mut m = Mock::new();
        m.regs[0] = 0x07;
        m.regs[1] = 0x3A;
        m.regs[reg::STATUS1 as usize] = status::SW_CC; // 電源投入時のキャリブレーション完了
        let mut p = Pinnacle::new(m);
        let info = p.init_relative(&mut NoDelay, RelativeOptions::default()).unwrap();
        assert_eq!(info.firmware_id, 0x07);
        let m = p.release();
        assert!(m.writes.contains(&(reg::SYS_CONFIG1, SYS_RESET)));
        // GlideExtend・スクロール・二次タップ無効、タップ有効、IntelliMouse なし
        assert_eq!(m.regs[reg::FEED_CONFIG2 as usize], 0x1C);
        // 相対モードで有効（パッド側の反転なし）
        assert_eq!(m.regs[reg::FEED_CONFIG1 as usize], 0x01);
        assert_eq!(m.regs[reg::Z_IDLE as usize], 0);
        assert_eq!(m.era[0x0187] & 0xC0, 0xC0, "sensitivity 4x");
    }

    #[test]
    fn parses_absolute_packet_and_masks_bit11() {
        let a = parse_absolute(&[0, 0, 0x34, 0x12, 0x85, 0x0D]);
        assert_eq!((a.x, a.y, a.z), (0x534, 0x012, 0x0D));
        // 4 バイト目の bit7（Y の bit11 に見えるフラグ）は無視する
        let a = parse_absolute(&[0, 0, 0, 0x52, 0xA2, 0x0D]);
        assert_eq!(a.y, 0x252);
    }

    #[test]
    fn set_absolute_toggles_only_the_mode_bit() {
        let mut m = Mock::new();
        m.regs[reg::FEED_CONFIG1 as usize] = 0x01 | 0x40;
        let mut p = Pinnacle::new(m);
        p.set_absolute(&mut NoDelay, true, 5).unwrap();
        assert_eq!(p.spi_mut().regs[reg::FEED_CONFIG1 as usize], 0x03 | 0x40);
        assert_eq!(p.spi_mut().regs[reg::Z_IDLE as usize], 5);
        p.set_absolute(&mut NoDelay, false, 0).unwrap();
        assert_eq!(p.spi_mut().regs[reg::FEED_CONFIG1 as usize], 0x01 | 0x40);
        assert_eq!(p.spi_mut().regs[reg::Z_IDLE as usize], 0);
    }

    #[test]
    fn read_absolute_reads_six_bytes_and_clears() {
        let mut m = Mock::new();
        m.regs[reg::STATUS1 as usize] = status::SW_DR;
        m.regs[0x12..0x18].copy_from_slice(&[0, 0, 0x00, 0x04, 0x34, 0x20]);
        let mut p = Pinnacle::new(m);
        let a = p.read_absolute().unwrap().unwrap();
        assert_eq!((a.x, a.y, a.z), (0x400, 0x304, 0x20));
        assert_eq!(p.read_absolute().unwrap(), None, "flags were cleared");
    }

    #[test]
    fn init_relative_rejects_wrong_firmware_id() {
        let mut m = Mock::new();
        m.regs[0] = 0xFC; // 配線が外れて詰め物のバイトが読めている
        let mut p = Pinnacle::new(m);
        assert!(matches!(p.init_relative(&mut NoDelay, RelativeOptions::default()), Err(Error::BadId(0xFC))));
        // 何も書き込んでいない
        assert!(p.release().writes.is_empty());
    }

    #[test]
    fn init_relative_options() {
        let mut m = Mock::new();
        m.regs[0] = 0x07;
        let mut p = Pinnacle::new(m);
        let opt =
            RelativeOptions { primary_tap: false, invert_y: false, invert_x: true, swap_xy: true, z_idle: 5, ..Default::default() };
        p.init_relative(&mut NoDelay, opt).unwrap();
        let m = p.release();
        assert_eq!(m.regs[reg::FEED_CONFIG2 as usize], 0x1C | 0x02 | 0x80);
        assert_eq!(m.regs[reg::FEED_CONFIG1 as usize], 0x01 | 0x40);
        assert_eq!(m.regs[reg::Z_IDLE as usize], 5);
    }

    #[test]
    fn read_relative_reads_one_packet() {
        let mut m = Mock::new();
        m.regs[reg::STATUS1 as usize] = status::SW_DR;
        m.regs[0x12..0x15].copy_from_slice(&[0x01, 7, 2]);
        let mut p = Pinnacle::new(m);
        let r = p.read_relative().unwrap().unwrap();
        assert_eq!((r.buttons, r.dx, r.dy), (1, 7, 2));
        assert_eq!(p.read_relative().unwrap(), None, "flags were cleared");
    }

    #[test]
    fn status_bits_match_hardware() {
        assert_eq!(status::SW_DR, 0x04);
        assert_eq!(status::SW_CC, 0x08);
    }

}
