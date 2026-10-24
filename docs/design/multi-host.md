# 複数のホスト（Mac・Windows）への切り替え

**状態（2026-10-24）: 方式を決めてファームを実装（実機は未確認）。Windows アプリは C# / WPF で `win/` に作成中。**

要望: Mac と Windows を両方使うので、キーボード（とトラックパッド）の送り先を切り替えたい。
瞬時に切り替えることより、**接続の安定性と消費電力**を重視する（持ち主、2026-10-24）。

## 決めた方式: 送り先ごとに別の機器として、1 台ずつつなぐ

| 送り先（プロファイル） | 名前 | BLE アドレス | 想定 |
|---|---|---|---|
| 1 台目 | `hikyaku` | `FF:E4:05:1A:8F:FF`（2026-10 までと同じ。Mac の再ペアリング不要） | Mac |
| 2 台目 | `hikyaku2` | `FF:E4:05:1A:8F:FE` | Windows |

- **一度につなぐのは 1 台だけ。** 切り替えると、今の接続を切って、送り先を保存して再起動し、その送り先のアドレス・名前・ペア情報で待つ。
- 送り先ごとにアドレスが違うので、ホストから見ると別の機器になる。Mac は hikyaku だけ、Windows は hikyaku2 だけを覚えていて、
  もう一方の送り先で待っている間につなぎに来ない（**取り合いが起きない**）。Logicool の Easy-Switch と同じ考え方。
- 切り替えにかかる時間は、再起動とつなぎ直しで数秒。送り先の切り替えでの再起動では、USB の書き換え猶予（3 秒）を飛ばす。

### 比べたもう一つの方式（採らなかった）

同じ hikyaku のまま 2 台に同時につなぎ、送り先だけ切り替える方式（ESP32-S3 は複数の接続ができる）。切り替えは一瞬だが、

- 2 台分の接続を保つので、無線の電力がほぼ 2 倍になる（使っていない側とも、接続のための通信を続ける）
- 2 つの接続の通信の時間がぶつかり、遅れや切断の原因になりうる
- 接続ごとの購読・送り先の管理など、ファームの変更が大きい

ので、安定性と電力を重視する今回の要望に合わない。

## 切り替えの操作

| 操作 | 動き |
|---|---|
| キーボードの **F21** / **F22** | 1 台目 / 2 台目へ（ホストには送らない。QMK で空いているキーに割り当てる） |
| 本体のボタンを、パッドを動かさずに **3 秒押し続ける** | もう一方の送り先へ（`hikyaku_pointer::button` の `LongHold`） |
| アプリの命令 `23 p` | 送り先 p へ（docs/design/ring.md） |

基板の RGB LED の、準備ができたときの色で送り先を示す: **1 台目 = 緑、2 台目 = 青**（2026-10-24。最初は点滅の回数にしたが、色の方が一目で分かる）。
パッドが見つからないときは、赤と送り先の色の交互の点滅。基板の青い点滅は充電の表示で、ファームとは関係ない。

## 送り先ごとに持つもの

- **ペア情報**（`bond_store`。フラッシュの形式 `HKB2` で送り先の番号つき。古い `HKB1` は 1 台目として読む）
- **スクロールの向き**（`settings` の `scroll_invert`。既定は 1 台目 = 反転（Mac の「ナチュラル」）、2 台目 = 反転しない（Windows の普通））。
  アプリの命令 `22 v` で変える
- 本体の置き方（向き）は共通（置き場所は同じなので）

## Windows のための変更

- **Device Information サービス（PnP ID）** を GATT の末尾に足した。HID over GATT の仕様で必須で、Windows はこれが無いとつながらないことがある。
  末尾なので既存のハンドルは動かず、Mac の再ペアリングは要らない。
- **2 台目だけ、GAP / GATT サービスを自前にした**（`windows_gap_table`。1 台目＝Mac は今までどおりで、ハンドルも動かない）。
  GATT サービス（0x1801）に Service Changed（0x2A05）を置き、ペアリングが済んで Windows が購読したら、全体（0x0001〜0xFFFF）が
  変わったと 1 回知らせる。GAP の Central Address Resolution も外した（周辺機器には不要）。下の「Windows で HID にならなかった件」
- 画面の明るさのメディアキー（Consumer 0x6F / 0x70）は Windows ではほとんど効かない。Windows アプリが WMI（内蔵画面）・DDC/CI（外部モニタ）で変える。

## Windows で HID にならなかった件（2026-10-24 に Windows で調べた）

症状: ペアリングでき「接続済み」になるが、キーもカーソルも動かない。

Windows 側で分かったこと（Windows 11 26200、アダプタは BARROT Bluetooth 6.0 Adapter。この PC で初めての BLE 機器）:

- ペアリングは成功している（Just Works・LE Secure Connections・鍵は IdKey だけ。Windows の記録でも `PairingResult` は 0）
- ところが Windows が作ったサービスの子は **0x1801（GATT サービス）だけ**（`BTHLEDevice\{00001801-...}_ffe4051a8ffe\...&0008`）。
  HID・電池・DIS の子は一度も作られない。ドライバの失敗ではない（setupapi.dev.log に失敗なし）
- BTHLE のノードのデバイスインターフェースが有効にならない（`{781aee18-...}` が Linked でない）。だから WinRT の
  `BluetoothLEDevice.FromIdAsync` / `FromBluetoothAddressAsync` が FileNotFoundException になる
- ETW（Microsoft の BluetoothStack.wprp）の HCI の記録では、ATT は「MTU 交換 → 0x1801 を Find By Type Value で探す
  （ハンドル 8〜8、中身なし）→ 続きを探す（Attribute Not Found）」だけ。そのあとペアリング・暗号化・鍵の配布まで進み、
  **暗号化の後は 20 秒間 ATT の要求が一つも出ない**（サービスの全体の発見をしない）。再接続でもしない
- Windows はペアリングの前に 0x1801 の中の Service Changed を探しに来ている。trouble-host の既定の 0x1801 は空

直し方（試す順）:

1. ファーム: 2 台目に Service Changed を置き、ペアリングの後に indication で発見をやり直させる（上の「Windows のための変更」）。
   **Windows で hikyaku2 を一度削除してから**ペアリングし直す（古い、0x1801 だけの情報を捨てるため）。
   hikyaku のログに `service changed: subscribed=true` → `asking the host to discover the services again` が出て、
   そのあと `gatt read handle=...`（Windows がレポートマップなどを読む）が続けば成功
   **結果（b00c509、21:54）: 直らなかった。** Windows は購読して indication に Confirmation を返すが、その後も 0x1801 の処理
   （Find By Type Value → Read By Type → Find Information → CCCD の書き込み）を繰り返すだけで、Read By Group Type を一度も出さない。
   hikyaku の応答はすべて規格どおり（ETW と hikyaku の ATT のログが 1 対 1 で一致）。Windows が 0x1801 の子を作ったのは
   indication より前で、そのとき Windows が知っていたのは 0x1801 だけ。Service Changed は規格どおりで害はないので、そのまま残す
2. **原因はアダプタだった（22:01 に確認）。** Linux の BlueZ で出した試験用の機器（hktest。BlueZ の既定の GATT に電池 0x180F を
   足しただけ）でも同じだった: 子は 0x1801 だけ、Read Remote Features は Controller Busy、Windows は Read By Group Type を出さない。
   アダプタは Barrot のドライバ（11.8.41.844、2025-02-12、oem90.inf）。ファームでは直せないので、別のアダプタ（Intel・Realtek など）
   を使うか、Barrot のドライバを更新する。以下はそのときの見立て:
   アダプタの問題を疑う。接続の直後に Windows が出した LE Read Remote Features を、アダプタが
   Controller Busy（0x3A）で断っている（アダプタが自分で Data Length の更新を始めたのとぶつかった）。別の Bluetooth アダプタ
   （Intel・Realtek など）で試すか、ほかの BLE 機器（BlueZ の周辺機器など）が同じアダプタで動くかを確かめる

## Windows アプリの技術選定: C# (.NET 8) + WPF

| 機能 | 方法 |
|---|---|
| BLE（独自サービス） | WinRT の `Windows.Devices.Bluetooth`（C# から直接使える）。HID として接続中の hikyaku の独自サービスを開けるかは要確認（Mac では開けた） |
| リングの表示 | WPF の透明・最前面のウィンドウに、`WS_EX_NOACTIVATE` / `WS_EX_TRANSPARENT`（フォーカスを奪わず、クリックを透過） |
| 常駐・ホットキー | タスクトレイ（WinForms の `NotifyIcon`）、`RegisterHotKey`（Ctrl+Alt+R） |
| 音量 | メディアキーを hikyaku から送る（Mac と同じ）。表示用の値は Core Audio（`IAudioEndpointVolume`） |
| 明るさ | WMI（`WmiMonitorBrightnessMethods`。内蔵画面）。外部モニタの DDC/CI は後回し |
| シーク | Windows のメディア操作（GSMTC: `GlobalSystemMediaTransportControlsSessionManager`）。アプリが位置の変更に応じるかは要確認 |
| 設定 | `%APPDATA%\HikyakuRing\config.json`（Mac と同じ形式） |

選ばなかったもの: WinUI 3（透明でクリックを透過するウィンドウが作りにくい）、Rust（UI と Windows の API を低い層から書くことになる）、
Tauri / Electron（常駐には重い）、Mac と共通の 1 つのアプリ（Mac アプリは完成済み）。

ビルドは Linux でもできる（.NET SDK の `EnableWindowsTargeting`。`win/build.sh` は Docker の .NET SDK を使う）。動かすのは Windows の実機。

## 確かめること（持ち主に頼む）

1. 新しいファームで、Mac（1 台目）が今までどおりつながる。LED が緑になる
2. F22（またはボタンの 3 秒長押し）で 2 台目へ。LED が青になり、`hikyaku2` として見える。Windows でペアリングして、キー・マウス・スクロールが動く
3. F21 で 1 台目へ戻ると、Mac に自動でつながる
4. Windows で `win/RingProbe`（独自サービスを開けるかの試作）が動く → 動けば Windows アプリ本体（`win/HikyakuRing`）を試す
