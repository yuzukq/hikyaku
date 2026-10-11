# ファームウェア

## 構成

```
firmware/
  hikyaku-s3/        ★本命。ESP32-S3 ボード: USB ホスト（キーボード）→ BLE HID
  hikyaku-c3/        初期検証用。XIAO ESP32-C3（こちらは XIAO）: PC から USB シリアルで受けた HID レポートを BLE へ
  uart-bridge-c3/    C3 を UART→USB 変換器にする（CH9102F があれば不要）
crates/hikyaku-hid/      no_std。ブートキーボードレポートの解釈・Usage 名（ホストでテスト可）
crates/hikyaku-pinnacle/ no_std。Cirque Pinnacle の SPI ドライバ（RAP, embedded-hal 1.0。SPI モックで単体テスト）
crates/hikyaku-pointer/  no_std。トラックパッドの絶対座標 → マウスレポート（移動・加速・平滑化・タップ・ドラッグ・スクロール）
tools/hid-probe/     PC 用: hidraw の一覧・記述子ダンプ・入力解析・ESP への中継（bridge）
tools/scripts/       検証用スクリプト（ble_rx.py, uart_cap.py）
```

言語は Rust（esp-hal 1.2 + esp-rtos + embassy + trouble）。

### hikyaku-s3 の中身（`src/bin/main.rs`）

- **`usb_host` タスク:** `embassy-usb-host` + esp-hal の OTG ホストドライバ。起動後 3 秒待ってから USB ホストを有効化（書き換え猶予）。
  IF0（ブートキーボード）を `KbdHandler` で、IF2（マウス・コンシューマ）を自前の `open_media_pipe` で読み、`REPORTS` チャネルへ流す。切断時は全解放レポートを送る。
- **BLE HID:** trouble の GATT サーバ。HID サービス（固定レポート記述子 149 バイト: キーボード ID1 / マウス ID2 / コンシューマ ID3）とバッテリーサービス。
  アドレスは固定のランダム静的アドレス `FF:E4:05:1A:8F:FF`。名前は `hikyaku`。IO 能力は NoInputNoOutput（JustWorks）。
- **ボンディング保存（`src/bin/bond_store/`）:** `conn.set_bondable(true)` した上で、`PairingComplete` の鍵をフラッシュの 0x9000（`nvs` 領域, 4KB）に保存。最大 4 台、起動時に復元。
  ※ `espflash erase-flash` すると消える（Mac 側でも削除して再ペアリングが必要）。
- **トラックパッド（`--features trackpad`、既定は無効）:** `src/bin/trackpad/mod.rs`。SPI2（SCK=GPIO12, MISO=GPIO13, MOSI=GPIO11, SS=GPIO10, DR=GPIO9, モード 1, 1MHz）で Pinnacle を読み、
  `hikyaku-pointer` でマウスレポートにして BLE の ID2 へ流す。`SCROLL_MODE`（AtomicBool）が true の間はスクロールに変換。**実機は未確認（コネクタ待ち）。**
  起動時のログ `trackpad: firmware id=... version=...` で疎通を確認する（参考実装では 0x07 / 0x3A）。
- **ログ:** `esp-println` を `uart` feature にして UART0（GPIO43 = ボードの TX ピン）へ。USB シリアルはホスト動作中に使えないため。

## ビルド

```fish
# S3（Xtensa）: espup の環境変数が必要
. ~/export-esp.sh
cd firmware/hikyaku-s3
cargo build --release                      # 既定
cargo build --release --features trackpad  # トラックパッド有効（同じ出力先を上書きするので、書き込む前に目的のビルドをやり直す）

# C3 / ホスト側ツール
cargo build --release      # firmware/hikyaku-c3 など
cargo test                 # リポジトリ直下: crates/hikyaku-{hid,pinnacle,pointer}, tools/hid-probe（計 27 件）
```

初回のセットアップ:

```fish
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh        # Rust
cargo install espflash espup esp-generate --locked
GITHUB_TOKEN=(gh auth token) espup install --targets esp32s3 --export-file ~/export-esp.sh
rustup target add riscv32imc-unknown-none-elf                          # C3 を触る場合
```

`espup install` は GitHub API のレート制限（403）にかかるので `GITHUB_TOKEN` を渡す。

## 現在 S3 に書き込まれているファーム

メディアキー対応版（`4818abd` 相当、`trackpad` feature なし）。ボンディング保存つき。
`firmware/hikyaku-s3/target/` の成果物は、最後にビルドしたものに上書きされる（`--features trackpad` を試すと、そちらになる）。
書き込む前に、目的のビルドをやり直すこと。

既知の課題: `forward report` / `gatt read|write` のログが `info!` で出ていて冗長（打鍵のたびに出る）。次に書き込むときに `debug!` へ下げる。

## 書き込み（S3）

S3 は 3 秒後に USB ホストになり、USB シリアル/JTAG が消える。

1. 5V 給電とキーボードを S3 から外す
2. S3 の **BOOT を押しながら** USB-C を PC に挿す
3. `cd firmware/hikyaku-s3 && espflash flash target/xtensa-esp32s3-none-elf/release/hikyaku-s3`
4. 抜いて、キーボードと給電を元に戻す

## ログを見る

CH9102F（または UART→USB 変換器）を S3 の TX（GPIO43）につなぎ、`tools/scripts/uart_cap.py 60 /dev/ttyACM0`。
期待するログ: `usb: waiting for device` → `usb: device 04d8:eb5f` → `usb: keyboard ready` → `usb: media interface 2`、接続時は `connected` / `pairing complete: ... bonded=true` / `bond_store: saved ...`。

## PC を BLE の受信側にして検証する

`tools/scripts/ble_rx.py 60` を走らせ、7skb で打鍵する。届いたレポートが 16 進で出る。

## 落とし穴（ハマったこと）

- **ESP の USB シリアルを開くとチップがリセットされる**（C3/S3 の USB-Serial-JTAG）。開いたまま使い、開いた後に ESP が起動し直すのを待つ。`hid-probe bridge` は開いた直後に DTR/RTS を同時に落として回避している。
- **`/dev/hidraw*` が root 専用:** udev ルール `/etc/udev/rules.d/70-hid-debug.rules` に
  `KERNEL=="hidraw*|event*", SUBSYSTEM=="hidraw|input", TAG+="uaccess"` を入れた。**番号は 73 未満**にする（`73-seat-late.rules` が ACL を付与するため、99 だと間に合わない）。`/dev/input/event*` は新しく作られるデバイスには効かなかったので、受信確認は hidraw を使う。
- **Python で hidraw を読むときは `buffering=0`:** バッファ付きだと `read(64)` が 64 バイト溜まるまで戻らず、9 バイトのレポートが見えない。
- **`bluetoothctl` の `pair` 直後に hidraw ができるまで少しかかる**（権限が付くのも少し遅れる）。リトライする。
- **`pkill -f`** は自分自身のコマンドラインにもマッチして自分を落とす。`pkill -x` を使う。
- **`conn.set_bondable(true)` を呼ばないと `PairingComplete` の `bond` が None** になり、保存されず再接続のたびに再ペアリングになる。
- **trouble の `notify` は未購読だと黙って成功を返す。** 届かないときは GATT の CCCD 書き込み（購読）をログで確認する。
- `src/bin/` 直下の `*.rs` は別バイナリ扱いになる。モジュールは `src/bin/<name>/mod.rs` に置く。
- S3 の USB ホスト中は USB シリアル/JTAG が使えない（同じ PHY を共有）。
