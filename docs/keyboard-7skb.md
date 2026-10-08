# 対象キーボード: Salicylic_Acid 7skb（7sPro）

QMK 系（レイヤーあり）。USB Type-C。VID:PID = **04d8:eb5f**。

## USB の構成（HID 3 インターフェース）

Linux の `hid-probe list` / `hid-probe desc N` で採取したレポート記述子から。

| IF | 役割 | 記述子 | 備考 |
|---|---|---|---|
| IF0 | **ブートキーボード**（subclass 1 / protocol 1） | 68 バイト | レポート ID なし。8 バイト入力 + LED 出力 |
| IF1 | QMK raw HID（vendor 0xFF60, usage 0x61） | 34 バイト | VIA 等の設定用。中継には不要 |
| IF2 | マウス / システム / コンシューマ（レポート ID 付き） | 123 バイト | 下記 |

### IF0 入力レポート（8 バイト, 6KRO）

| バイト | 内容 |
|---|---|
| 0 | 修飾キー（bit0 LCtrl, 1 LShift, 2 LAlt, 3 LGui, 4 RCtrl, 5 RShift, 6 RAlt, 7 RGui） |
| 1 | 予約 |
| 2〜7 | 押されているキーの Usage ID（最大 6 個。0=なし, 1=ロールオーバーエラー） |

### IF2 の入力レポート（先頭 1 バイトがレポート ID）

| ID | 内容 | 長さ（ID 込み） | 備考 |
|---|---|---|---|
| 2 | マウス: buttons(8bit), X, Y, wheel, AC Pan（いずれも int8 -127〜127） | 6 | |
| 3 | システムコントロール（電源/スリープ等）16bit usage, 範囲 1〜0xB7 | 3 | **BLE 側の記述子に無いので転送していない** |
| 4 | コンシューマ（音量・輝度など）16bit usage, 範囲 1〜0x2A0 | 3 | BLE 側では ID3 に付け替えて送る |

## 注意

- レイヤー 2 の「GC SWAP」を押していると Ctrl と Cmd が入れ替わる（Ctrl+→ が効かないように見えた原因）。
- ブートプロトコルを選んで読んでいる（NKRO 非対応のキーボードなら足りる）。

## BLE 側（hikyaku）に出している固定記述子

接続元のキーボードによらず同じ記述子を出す（macOS のペアリングキャッシュ対策）。

| ID | 内容 | 長さ |
|---|---|---|
| 1 | キーボード（上記 IF0 と同じ 8 バイト入力 + LED 出力 1 バイト） | 8 |
| 2 | マウス: buttons, X, Y, wheel, pan | 5 |
| 3 | コンシューマ（16bit usage） | 2 |
