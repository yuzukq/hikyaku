# 配線

## XIAO ESP32-S3 のピン対応

| XIAO | GPIO | 用途 |
|---|---|---|
| D0 | 1 | トラックパッド DR（予定） |
| D1 | 2 | トラックパッド SS（予定） |
| D2 | 3 | （空き。ストラッピングピンなので避ける） |
| D3〜D5 | 4〜6 | 空き（D4=SDA, D5=SCL にもなる） |
| D6 | 43 | UART0 TX = **ログ出力**（開発中） |
| D7 | 44 | UART0 RX（未使用） |
| D8 | 7 | SPI SCK（予定。トラックパッド） |
| D9 | 8 | SPI MISO（予定。トラックパッド） |
| D10 | 9 | SPI MOSI（予定。トラックパッド） |
| USB-C | 19/20 | **USB ホスト**（D−=GPIO19, D+=GPIO20）。キーボードを接続 |
| 5V | — | VBUS と直結。外部から給電 → キーボードにも供給される |
| 3V3 | — | トラックパッドの電源（2.9mA） |

## 開発中の構成（現在）

```
PC ─USB─ CH9102F ─ +5V ──▶ S3 5V ─(S3 の USB-C)─ C-C ケーブル ─ 7skb
                 ─ GND ──▶ S3 GND
                 ─ RxD ◀── S3 D6 (TX)
                         S3 ──BLE──▶ Mac
```

- S3 の USB-C は **PC につながない**（キーボード専用）。
- ログは PC 側の CH9102F のシリアルポートで読む。
- 変換器を C3 で代用していた頃は、C3 の D7(GPIO20) が RX、5V ピンから S3 へ給電していた。

## 書き換えるとき

S3 は起動して 3 秒後に USB ホストになり、USB シリアル/JTAG（書き込み用）が使えなくなる。

1. 給電（CH9102F またはバッテリー）の 5V を S3 から外し、キーボードも外す
2. S3 の **BOOT を押しながら** USB-C を PC に挿す（ダウンロードモード）
3. `espflash flash ...`（[../firmware.md](../firmware.md)）
4. 抜いて、キーボードと給電を元に戻す

## 最終形（予定）

```
モバイルバッテリー ─ Type-C DIP 化キット(+CC 5.1kΩ×2) ─▶ S3 5V/GND ─ 7skb
トラックパッド TM035035 ─ SPI ─ S3（SCK=D8, MISO=D9, MOSI=D10, SS=D1, DR=D0, 3V3, GND）
S3 ──BLE──▶ Mac
```

トラックパッドのピン順は [trackpad-tm035035.md](trackpad-tm035035.md)、電源は [power.md](power.md)。
