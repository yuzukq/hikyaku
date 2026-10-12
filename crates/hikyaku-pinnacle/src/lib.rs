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
/// FeedConfig1: 絶対座標モード
const FEED_ABSOLUTE: u8 = 1 << 1;
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
}

impl Default for RelativeOptions {
    /// Cirque 公式の USB マウスのサンプルと同じ（Y を反転して画面の向きに合わせる、タップ有効、感度 4X）。
    fn default() -> Self {
        Self { sensitivity: Attenuation::X4, primary_tap: true, invert_x: false, invert_y: true, swap_xy: false }
    }
}

/// `read_absolute_checked` の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checked {
    pub data: AbsoluteData,
    /// 読んだ生のパケット（レジスタ 0x12〜0x17）。診断用。
    pub raw: [u8; 6],
    /// 2 回読んだ結果が食い違った（3 回目で多数決した）。SPI 読み取りの誤りの目安。
    pub disagreed: bool,
}

/// `init` の結果。ファームウェア ID/バージョンで疎通を確認する（参考実装では ID=0x07, Ver=0x3A）。
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

    /// 初期化して絶対座標モードで動かす（QMK の `cirque_pinnacle_init` と同じ手順）。ID とバージョンを返す。
    ///
    /// リセット → 30ms 待つ → 設定 → ADC 減衰（ERA）→ キャリブレーション → Feed 有効化。
    /// 実機で、リセット・ADC 減衰・キャリブレーションを省くと、座標の上位ビットの更新が遅れてカーソルが飛んだため。
    /// **起動時（キャリブレーション中）はパッドに触れないこと**（補正が狂う）。
    pub fn init<D: DelayNs>(&mut self, delay: &mut D, attenuation: Attenuation) -> Result<Info, Error<SPI::Error>> {
        let firmware_id = self.read_reg(reg::FIRMWARE_ID)?;
        let firmware_version = self.read_reg(reg::FIRMWARE_VERSION)?;

        // 念のためリセット（電源を入れ直さずにマイコンだけ再起動した場合のため）。起動に 10〜15ms かかる。
        self.write_reg(reg::SYS_CONFIG1, SYS_RESET)?;
        delay.delay_ms(30);
        self.write_reg(reg::SYS_CONFIG1, 0x00)?;
        delay.delay_us(50);
        self.clear_flags_wait(delay)?;

        // 絶対座標モードでは FeedConfig2 は既定値（0）。データ形式は絶対座標。Feed はまだ有効にしない。
        self.write_reg(reg::FEED_CONFIG2, 0x00)?;
        self.write_reg(reg::FEED_CONFIG1, FEED_ABSOLUTE)?;
        self.write_reg(reg::Z_IDLE, Z_IDLE_COUNT)?;

        // ADC の減衰を設定し、変えたらキャリブレーションする。
        if self.set_adc_attenuation(delay, attenuation)? {
            self.calibrate(delay)?;
        }

        self.enable_feed(true)?;
        Ok(Info { firmware_id, firmware_version })
    }

    /// 相対モードで初期化する（Zephyr / ZMK の `pinnacle_init` と同じ手順）。
    ///
    /// ソフトリセット → 電源投入時のキャリブレーション完了（SW_CC）を待つ → 感度（ADC 減衰）→ キャリブレーション →
    /// FeedConfig2（GlideExtend・スクロール・二次タップ無効。IntelliMouse は使わない＝3 バイトのパケット）→
    /// FeedConfig1（相対モードで有効化 + 反転）→ ZIdle = 0。
    /// **起動中（約 0.5 秒）はパッドに触れないこと**（補正が狂う）。
    pub fn init_relative<D: DelayNs>(&mut self, delay: &mut D, opt: RelativeOptions) -> Result<Info, Error<SPI::Error>> {
        let firmware_id = self.read_reg(reg::FIRMWARE_ID)?;
        let firmware_version = self.read_reg(reg::FIRMWARE_VERSION)?;

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
        self.write_reg(reg::Z_IDLE, 0)?;
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

impl<SPI: SpiDevice> Pinnacle<SPI> {
    /// `read_absolute` と同じだが、パケットを 2 回読んで比べ、食い違ったら 3 回目を読んでビットごとに多数決する。
    ///
    /// 実機で、座標の bit7（±128）が一部のサンプルで反転する誤りがあった（Y で約 5%）ため。
    /// データが凍結されている間（フラグをクリアするまで）に読むので、複数回読んでも同じパケットが得られる。
    pub fn read_absolute_checked(&mut self) -> Result<Option<Checked>, Error<SPI::Error>> {
        let st = self.read_reg(reg::STATUS1)?;
        if st & (status::SW_DR | status::SW_CC) == 0 {
            if st != 0 {
                self.clear_flags()?;
            }
            return Ok(None);
        }
        let mut a = [0u8; 6];
        let mut b = [0u8; 6];
        self.read_regs(reg::PACKET_BYTE_0, &mut a)?;
        self.read_regs(reg::PACKET_BYTE_0, &mut b)?;
        let (packet, disagreed) = if a == b {
            (a, false)
        } else {
            let mut c = [0u8; 6];
            self.read_regs(reg::PACKET_BYTE_0, &mut c)?;
            let mut m = [0u8; 6];
            for i in 0..6 {
                m[i] = (a[i] & b[i]) | (a[i] & c[i]) | (b[i] & c[i]);
            }
            (m, true)
        };
        self.clear_flags()?;
        Ok(Some(Checked { data: parse_absolute(&packet), raw: packet, disagreed }))
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
    fn init_follows_the_qmk_sequence() {
        let mut m = Mock::new();
        m.regs[0] = 0x07;
        m.regs[1] = 0x3A;
        let mut p = Pinnacle::new(m);
        let info = p.init(&mut NoDelay, Attenuation::X4).unwrap();
        assert_eq!(info, Info { firmware_id: 0x07, firmware_version: 0x3A });
        let m = p.release();
        // リセットを送ってから既定に戻している
        let pos_reset = m.writes.iter().position(|&w| w == (reg::SYS_CONFIG1, SYS_RESET)).expect("reset");
        assert_eq!(m.writes[pos_reset + 1], (reg::SYS_CONFIG1, 0x00));
        // 絶対座標モード、FeedConfig2 は既定値、ZIdle=5
        assert_eq!(m.regs[reg::FEED_CONFIG2 as usize], 0x00);
        assert_eq!(m.regs[reg::Z_IDLE as usize], 5);
        // ADC の減衰が 4X（0xC0）になり、他のビット（0x4E の下位）は保たれる
        assert_eq!(m.era[0x0187], 0xCE);
        // 減衰を変えたのでキャリブレーションした
        assert_eq!(m.calibrations, 1);
        // 最後は Feed 有効（絶対モードのまま）
        assert_eq!(m.regs[reg::FEED_CONFIG1 as usize], FEED_ENABLE | FEED_ABSOLUTE);
        let feed_on = m.writes.iter().rposition(|&w| w == (reg::FEED_CONFIG1, FEED_ENABLE | FEED_ABSOLUTE)).unwrap();
        let cal = m.writes.iter().position(|&w| w.0 == reg::CAL_CONFIG1).unwrap();
        assert!(feed_on > cal, "feed is enabled after calibration");
    }

    #[test]
    fn init_skips_calibration_when_attenuation_already_matches() {
        let mut m = Mock::new();
        m.era[0x0187] = 0xCE; // すでに 4X
        let mut p = Pinnacle::new(m);
        p.init(&mut NoDelay, Attenuation::X4).unwrap();
        assert_eq!(p.release().calibrations, 0);
    }

    #[test]
    fn era_write_and_read_round_trip() {
        let mut p = Pinnacle::new(Mock::new());
        p.era_write(&mut NoDelay, 0x0149, 0x04).unwrap();
        assert_eq!(p.era_read(&mut NoDelay, 0x0149).unwrap(), 0x04);
        assert_eq!(p.era_read(&mut NoDelay, 0x0187).unwrap(), 0x4E);
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

    /// 読み出しのたびに、決まった番号の読み出し（0 始まり、データ読み出しのみ数える）でビットを反転させるモック。
    struct Flaky {
        inner: Mock,
        reads: usize,
        /// (何回目の読み出しか, 読み出し結果の何バイト目か, XOR するマスク)
        corrupt: std::vec::Vec<(usize, usize, u8)>,
    }

    impl ErrorType for Flaky {
        type Error = Infallible;
    }

    impl SpiDevice for Flaky {
        fn transaction(&mut self, ops: &mut [Operation<'_, u8>]) -> Result<(), Infallible> {
            self.inner.transaction(ops)?;
            for op in ops {
                if let Operation::Transfer(rx, tx) = op {
                    // 6 バイトのパケット読み出し（3 + 6）だけを数える
                    if rx.len() == 9 && tx[0] == (0xA0 | reg::PACKET_BYTE_0) {
                        for &(n, byte, mask) in &self.corrupt {
                            if n == self.reads {
                                rx[3 + byte] ^= mask;
                            }
                        }
                        self.reads += 1;
                    }
                }
            }
            Ok(())
        }
    }

    fn flaky(corrupt: &[(usize, usize, u8)]) -> Pinnacle<Flaky> {
        let mut m = Mock::new();
        m.regs[reg::STATUS1 as usize] = status::SW_DR;
        m.regs[0x12..0x18].copy_from_slice(&[0, 0, 0x7F, 0xFB, 0x12, 0x0C]);
        Pinnacle::new(Flaky { inner: m, reads: 0, corrupt: corrupt.to_vec() })
    }

    #[test]
    fn checked_read_agrees_without_errors() {
        let mut p = flaky(&[]);
        let c = p.read_absolute_checked().unwrap().unwrap();
        assert!(!c.disagreed);
        assert_eq!((c.data.x, c.data.y), (0x27F, 0x1FB & 0x7FF));
    }

    #[test]
    fn checked_read_fixes_a_flipped_bit7_by_majority() {
        // 1 回目の読み出しで Y 下位バイト（3 バイト目）の bit7 が反転した。
        let mut p = flaky(&[(0, 3, 0x80)]);
        let c = p.read_absolute_checked().unwrap().unwrap();
        assert!(c.disagreed);
        assert_eq!(c.data.y, 0x1FB, "majority restores the real value");
    }

    #[test]
    fn checked_read_fixes_an_error_in_the_second_read_too() {
        let mut p = flaky(&[(1, 2, 0x80)]);
        let c = p.read_absolute_checked().unwrap().unwrap();
        assert!(c.disagreed);
        assert_eq!(c.data.x, 0x27F);
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
        // 相対モードで有効 + Y 反転（Cirque の USB マウスのサンプルと同じ 0x81）
        assert_eq!(m.regs[reg::FEED_CONFIG1 as usize], 0x81);
        assert_eq!(m.regs[reg::Z_IDLE as usize], 0);
        assert_eq!(m.era[0x0187] & 0xC0, 0xC0, "sensitivity 4x");
    }

    #[test]
    fn init_relative_options() {
        let mut p = Pinnacle::new(Mock::new());
        let opt = RelativeOptions { primary_tap: false, invert_y: false, invert_x: true, swap_xy: true, ..Default::default() };
        p.init_relative(&mut NoDelay, opt).unwrap();
        let m = p.release();
        assert_eq!(m.regs[reg::FEED_CONFIG2 as usize], 0x1C | 0x02 | 0x80);
        assert_eq!(m.regs[reg::FEED_CONFIG1 as usize], 0x01 | 0x40);
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
