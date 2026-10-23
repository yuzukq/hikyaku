# PCB 化

ユニバーサル基板（[assembly.md](assembly.md)）の手配線では、取り違え・短絡・接触不良が多かった。
モジュールはそのまま使い、**ピンソケットで受けるだけの基板**を作って、小型化とはんだ付けの手間を減らす。
ESP32-S3・CH9102F・FFC 変換基板の部品を 1 枚に移植することはしない。

- KiCad 10 のプロジェクト: `pcb/hikyaku.kicad_pro`（回路図 `hikyaku.kicad_sch`、基板 `hikyaku.kicad_pcb`）
- 状況（2026-10-23）: 回路図とフットプリントの割り当てまで済んだ。持ち主がレビュー中。レイアウトはケースの設計に合わせて持ち主が行う。

## 部品とフットプリント

| Ref | 部品 | フットプリント | 備考 |
|---|---|---|---|
| J1 | S3 SuperMini の左列（TX, RX, GP1〜GP7） | `Connector_PinSocket_2.54mm:PinSocket_1x09_P2.54mm_Vertical` | 1 番 = TX（USB-C 側） |
| J2 | S3 SuperMini の右列（5V, GND, 3V3, GP13〜GP8） | 同上 | 1 番 = 5V（USB-C 側）。**J1 との間隔は中心で 15.24mm**。部品面から見て J1 が左 |
| J3 | FFC 12P 変換基板 | `Connector_PinSocket_2.54mm:PinSocket_2x06_P2.54mm_Vertical` | 2 列 × 6 本。向きは下記 |
| J4 | 秋月 AE-CH9102F-TYPEC-BO | `Connector_PinSocket_2.54mm:PinSocket_2x04_P2.54mm_Vertical` | CN2 と CN3 をまとめて受ける |
| SW1 | MX スイッチ（GPIO6） | `kbd_SW:CherryMX_Hotswap_1u` | Kailh ソケットのパッドは裏面（B.Cu） |
| SW2 | MX スイッチ（GPIO7） | 同上 | **ファームは未対応** |

`kbd_SW` は Salicylic Acid さんのライブラリ（`~/develop/custom-keyboard/kicad/libs/KiCAD_FootPrint/kbd_SW.pretty`）。
KiCad のグローバルのフットプリントライブラリに登録してある前提。ほかは KiCad 標準のライブラリ。

## 接続

| ネット | つながるピン |
|---|---|
| `+5V` | J2-1（S3 5V）、J4-7（CH9102F +5V） |
| `GND` | J2-2、J3-2、J4-5、SW1-2、SW2-2 |
| `+3V3` | J2-3（S3 3V3）、J3-1（トラックパッド VDD） |
| `TP_SCK` | J2-5（GPIO12）、J3-12 |
| `TP_MISO` | J2-4（GPIO13）、J3-11 |
| `TP_SS` | J2-7（GPIO10）、J3-10 |
| `TP_DR` | J2-8（GPIO9）、J3-9 |
| `TP_MOSI` | J2-6（GPIO11）、J3-8 |
| `LOG_TX` | J1-1（TX = GPIO43）、J4-1（CH9102F RxD） |
| `SW1` | J1-8（GPIO6）、SW1-1 |
| `SW2` | J1-9（GPIO7）、SW2-1 |

ほかのピンは未接続（回路図では NC の印）。特に **CH9102F の TxD（J4-3）はつながない**（5V 出力。S3 の GPIO は 5V 耐性なし）。

**5V のジャンパーはない**（ユニバーサル基板と同じく直結）。書き込むときは CH9102F の USB を抜いてから S3 を PC に挿す。
必要になったら、J2-1 と J4-7 の間に 1×2 のピンヘッダを入れる。

### J3（FFC 変換基板）の向き

**パッド k 番 = 変換基板の印字 k 番 = トラックパッド J1 の (13 − k) 番。** 1 番（四角パッド）が VDD（3V3）、12 番が SCK。

手元の組み合わせ（同方向の FFC + 変換基板）では、変換基板の印字と J1 の番号が逆順になる（[assembly.md](assembly.md) の注意）。
ユニバーサル基板では変換基板を 180° 回して挿して動かしていた。PCB ではこの対応を配線に織り込んであるので、**変換基板は印字の 1 番を四角パッドに合わせて挿す**。
FFC ケーブルの種類を変えると、また逆になる可能性がある。

### J4（CH9102F）のピン

説明書（[usb-serial-ch9102f.md](usb-serial-ch9102f.md)）では、CN2 と CN3 は 2.54mm 間隔で並び、**番号の向きが互い違い**（CN2 の 1 番 VIO の隣が CN3 の 4 番 RxD）。
部品面を上にしたとき、2×4 のソケットの奇数側が CN3、偶数側が CN2 になる:

| パッド | 1 | 3 | 5 | 7 | | 2 | 4 | 6 | 8 |
|---|---|---|---|---|---|---|---|---|---|
| 信号 | RxD | TxD | GND | +5V | | VIO | DTR | RTS | CTS |

## 発注前の確認（1:1 印刷）

2 列のヘッダは、裏表の取り違え（左右反転）が起きやすい。レイアウトができたら、F.Cu と F.SilkS を**原寸で印刷**し、実物を置いて確かめる。

- **J3:** 変換基板の印字 1 番を四角パッドに合わせて、12 本とも穴に乗るか。片方の列がはみ出すなら左右が逆なので、奇数と偶数の組（1⇔2、3⇔4 …）を入れ替える。FFC ケーブルがどちらに出るか（ケースの設計に関わる）もここで分かる。
- **J4:** 部品面を上にして、RxD が四角パッド、その隣が VIO、反対の端が +5V と CTS になるか。
- **J1 / J2:** 列の間隔が 15.24mm で、部品面から見て TX の列が左、どちらも 1 番が USB-C 側か（[wiring.md](wiring.md) のピン配置）。
- **初めて電源を入れる前:** J3 の 1 番とトラックパッドの VDD パッドが導通すること。

## KiCad の作業

- 回路図を変えたら、PCB エディタで **F8（回路図から PCB を更新）** してフットプリントとネットを反映する。
- `kicad-cli`（`/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli`）でできるのは、ERC / DRC、ネットリスト・BOM、ガーバー・ドリル・部品配置ファイル、PDF / SVG、STEP の出力。
  回路図の編集、フットプリントの配置、配線、F8 はできない。

```fish
set K /Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli
$K sch erc pcb/hikyaku.kicad_sch                 # 回路図のチェック
$K pcb drc pcb/hikyaku.kicad_pcb                 # 基板のチェック
$K pcb export gerbers -o out/gerber/ pcb/hikyaku.kicad_pcb
$K pcb export drill -o out/gerber/ pcb/hikyaku.kicad_pcb
```

- 最初の回路図は、`.kicad_sch` のテキストを直接書いて作り、ERC（0 件）とネットリストを [assembly.md](assembly.md) の配線表と突き合わせて確かめた。
- `pcb/hikyaku.kicad_prl`（個人の表示設定）はコミットしない。
