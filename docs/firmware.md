# ファームウェア

## 構成

```
firmware/
  hikyaku-s3/        ★本命。ESP32-S3 ボード: USB ホスト（キーボード）→ BLE HID
  hikyaku-c3/        初期検証用。XIAO ESP32-C3（こちらは XIAO）: PC から USB シリアルで受けた HID レポートを BLE へ
  uart-bridge-c3/    C3 を UART→USB 変換器にする（CH9102F があれば不要）
crates/hikyaku-hid/      no_std。ブートキーボードレポートの解釈・Usage 名（ホストでテスト可）
crates/hikyaku-pinnacle/ no_std。Cirque Pinnacle の SPI ドライバ（RAP, embedded-hal 1.0。SPI モックで単体テスト）
crates/hikyaku-pointer/  no_std。`relative`: 相対モードのパケット → マウスレポート（向き・倍率・持ち越し・分割。静止フィルタ・自前タップ判定）。
                         ホストで実機ログを再生してテストできる。旧方式（絶対座標）はタグ `absolute-mode-last` に残っている
tools/hid-probe/     PC 用: hidraw の一覧・記述子ダンプ・入力解析・ESP への中継（bridge）
tools/scripts/       検証用スクリプト（ble_rx.py, uart_cap.py）
```

言語は Rust（esp-hal 1.2 + esp-rtos + embassy + trouble）。

### hikyaku-s3 の中身（`src/bin/main.rs`）

- **`usb_host` タスク:** `embassy-usb-host` + esp-hal の OTG ホストドライバ。起動後 3 秒待ってから USB ホストを有効化（書き換え猶予）。
  IF0（ブートキーボード）を自前の `open_keyboard` で、IF2（マウス・コンシューマ）を `open_media_pipe` で開き、`REPORTS` チャネルへ流す。切断時は全解放レポートを送る。
  どちらも認識（`enumerate`）のときに読んだ構成記述子を使い、読み直さない（ライブラリの `KbdHandler` は読み直すところで TOFU60 が止まった）。
  バスリセット（つないだときも）の後は 100ms 待ってから問い合わせ、認識・制御転送には時間切れを付ける。認識に失敗したら、
  挿し直しを待たずにバスリセットで 3 回までやり直す（`USB_RETRIES` など。TOFU60 で確認。7sPro はこの版では未確認）。
- **BLE HID:** trouble の GATT サーバ。HID サービス（固定レポート記述子 149 バイト: キーボード ID1 / マウス ID2 / コンシューマ ID3）とバッテリーサービス。
  アドレスは固定のランダム静的アドレス `FF:E4:05:1A:8F:FF`。名前は `hikyaku`。IO 能力は NoInputNoOutput（JustWorks）。
- **ボンディング保存（`src/bin/bond_store/`）:** `conn.set_bondable(true)` した上で、`PairingComplete` の鍵をフラッシュの 0x9000（`nvs` 領域, 4KB）に保存。最大 4 台、起動時に復元。
  ※ `espflash erase-flash` すると消える（Mac 側でも削除して再ペアリングが必要）。
- **トラックパッド（`--features trackpad`、既定は無効）:** `src/bin/trackpad/mod.rs`。SPI2（SCK=GPIO12, MISO=GPIO13, MOSI=GPIO11, SS=GPIO10, DR=GPIO9, モード 1, **2MHz**）で Pinnacle を
  **相対モード**で読み、`hikyaku_pointer::relative::Relay` でマウスレポートにして BLE の ID2 へ流す。`SCROLL_MODE`（AtomicBool）が true の間はスクロールに変換。
  **相対モードで動作確認済み（2026-10-13）。** 起動時のログ `trackpad: relative mode, firmware id=0x07 version=0x3a` で疎通を確認する（ID が違えば止まる）。
  調整用の feature（どれも既定は無効。1 つずつ試す）:
  | feature | 内容 |
  |---|---|
  | `trackpad-still-filter` | 指を止めているときの ±1〜2 のぶれを抑える（動き始めは鈍らせない） |
  | `trackpad-soft-tap` | パッド側のタップを切り、ソフト側でタップを判定する（スワイプ中の誤クリック対策。ZIdle=5） |
  | `trackpad-sens-1x` / `-3x` / `-4x` | タッチ感度（ADC 減衰）。既定は **2X**（2026-10-15 に実機で決定。4X では指の検出が途切れた）。1X がいちばん敏感、4X がいちばん鈍い |
  | `trackpad-trace` | 生パケットをログに出す（ホストで再生できる。[../tools/traces/README.md](../tools/traces/README.md)） |
  | `trackpad-selftest` | 起動時に SPI の読み戻しテスト |
  | `usb-trace` | USB キーボードのレポートを 1 つずつログに出す（OTA の組み合わせキーの切り分け用） |
  | `trackpad-probe` | 起動時に DR・MISO のピンの状態と、GPIO9〜13 の短絡を調べてログに出す（テスターなしの配線の切り分け。診断のときだけ） |
  | `usb-log` | ログを S3 の USB-C に出す（キーボード用の USB ホストは止まる。下の「CH9102F なしでログを見る」） |
  | `serial-bridge` | 試験用。`usb-log` に加え、PC から USB シリアルで届くキー入力を USB キーボードの代わりに使う（下の「PC のキーボードで試す」） |

  1 秒ごとの集計ログ `trackpad: packets= reports= dropped= max_gap_ms= ...`: `dropped` は BLE へ送るキューがいっぱいで捨てた数、
  `max_gap_ms` は読み取りの間隔の最大（タスクが止まっていないか）。2026-10-15 の実機ではどちらも問題なし（0 と 6ms）。
- **LED（`src/bin/led/`）:** ボード（ESP32-S3 SuperMini）の GPIO48 の RGB LED（WS2812 互換）を RMT で駆動。トラックパッドの初期化中は**赤**（触らない。起動時のキャリブレーションが狂う）、
  終わったら**緑**、パッドが見つからない（ID が 0x07 でない）と**青**。LED は最後の色を保持するので、書き込み中や再起動の直前も前の色のまま。
  ボードの青い点滅は電池の充電表示（電池なし）で、ファームとは関係ない。
  USB キーボードの状態（診断用）: 認識したら**白**に一瞬、最初のキー入力で**水色**に一瞬。失敗したら色を残す（認識の失敗 = **黄**、ブートキーボードが無い = **紫**、読み取りエラー = **オレンジ**）。
- **スクロール:** キーボードの **F24** を押している間、トラックパッドの上下の動きを縦スクロールにする（`forward_key` で F24 を取り除き、`trackpad::SCROLL_MODE` を立てる）。
- **本体の置き方:** 時計回りに 0° / 90° / 180° / 270°（`hikyaku_pointer::Rotation`、`trackpad::ROTATION`）。カーソル・スクロールは向きの補正の後に回し、リングは角度に足す。
  起動時に `settings`（nvs の 0xA000 のセクタ）から読み、アプリの命令（ring.ctrl の `20 q`）で変えて保存する（docs/design/ring.md）。
- **本体のボタン:** GPIO6 と GND の間の MX スイッチ（内部プルアップ。押すと Low）。トラックパッドのタスクが 5ms ごとに読み、`hikyaku_pointer::button` で判定する。
  単押し = 右クリック（ダブルタップを待つ 250ms 後に出る）、押している間 = スクロール（押した瞬間から。押したまま動かしたら、離してもクリックしない）、
  ダブルタップ = リングの開閉（2 回目を押した瞬間。F23 と同じで、アプリが HELLO を送っていないと開かない）。リング中（ノブ操作を含む）は、1 回押せばすぐ閉じる（右クリック・スクロールにはしない）。
  時間は `trackpad/mod.rs` の `BUTTON_TIMING`（チャタリング 5ms・単押しの上限 200ms・ダブルタップの間隔 250ms）。パッドが見つからないとボタンも動かない。
- **ログ:** 既定（feature `log-uart`）は `esp-println` の UART0（GPIO43 = ボードの TX ピン）へ。USB シリアルはホスト動作中に使えないため。`usb-log` では USB シリアル/JTAG へ。

## ビルド

```fish
# S3（Xtensa）: espup の環境変数が必要
. ~/export-esp.sh
cd firmware/hikyaku-s3
cargo build --release                      # 既定
cargo build --release --features trackpad  # トラックパッド有効（同じ出力先を上書きするので、書き込む前に目的のビルドをやり直す）

# C3 / ホスト側ツール
cargo build --release      # firmware/hikyaku-c3 など
cargo test --workspace     # リポジトリ直下: crates/hikyaku-{hid,pinnacle,pointer}, tools/hid-probe（2026-10-13 時点で 79 件）
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

## 開発用 CLI（`tools/hk`）

```fish
tools/hk flash          # ビルド → S3 を待って書き込み（CH9102F の USB を抜いて、S3 を PC に挿す。BOOT は不要）
tools/hk log            # S3 のログを表示（CH9102F 経由）
tools/hk flash-log      # 書き込み → 元の構成に戻すのを待って → ログ表示
FEATURES="trackpad trackpad-trace" tools/hk flash   # 機能を変えてビルド
```

**CH9102F なしでログを見る（診断）:** `usb-log` を付けると、ログを S3 の USB-C（USB シリアル/JTAG）から PC に出す。
キーボード用の USB ホストは動かさない（7skb は使えない）。この版が入っている間は、BOOT を押さなくても `tools/hk flash` で書き込める。

```fish
FEATURES="trackpad usb-log trackpad-probe trackpad-selftest" tools/hk flash   # S3 を PC に直結（CH9102F の USB は抜く）
tools/hk usb-log 20                                                          # ログを 20 秒読む
```

**PC のキーボードで試す（キーボードが手元にないとき）:** `FEATURES="trackpad serial-bridge" tools/hk flash` を書き込み、S3 を PC に直結したまま
`cargo run -q -p hid-probe -- evbridge /dev/ttyACM0` を動かす。evdev（既定は keyd の仮想キーボード）のキー入力を S3 へ送り、S3 が BLE で Mac へ流す。
**Esc を 200ms 以上押すと F24**（スクロールの試験用）、短く押せば Esc。入力はデスクトップにも届くので、何も起きないウィンドウにフォーカスしておく。

`trackpad-probe` は起動時に、SPI を使う前のピンの状態（DR、MISO のプルアップ/プルダウン）と、GPIO9〜13 の短絡検査
（1 本ずつ駆動して、ほかのピンがつられるか・駆動したレベルにならないか）をログに出す。テスターなしで配線を切り分けられる
（変換基板の逆挿しを見つけた。[hardware/assembly.md](hardware/assembly.md)）。パッドの出力と一瞬ぶつかるので、診断のときだけ使う。

## 補助スクリプト（`tools/scripts/`）

- `flash_when_ready.sh` … S3 が PC に現れたら、すぐ有線で書き込む（BOOT を押しながら挿すタイミングを気にしなくてよい）。失敗しても再試行する。
- `capture_when_ready.sh <秒数> <出力>` … CH9102F が現れたら S3 のログを一定時間取る。
- `uart_cap.py <秒数> [ポート]` … ログのキャプチャ本体。`ble_rx.py` … PC を BLE の受信側にして、届くレポートを表示する。

## 書き込み（S3）

**OTA を使えば、以降は抜き差し・BOOT 操作が要らない → [ota.md](ota.md)。** 以下は有線で書き込む場合（最初の 1 回、または復旧）。

**BOOT は不要（2026-10-20 確認）:** `tools/hk flash` を実行してから S3 を PC に普通に挿すだけで書き込める。ファームは起動後 3 秒間 USB ホストにならず、その間は USB シリアル/JTAG として見えるので、espflash が自動で書き込みモードに入れる（書き込みモードに入った後は 3 秒の制限はない）。`usb-log` の版が入っているときは、挿しっぱなしでも書き込める。BOOT を押しながら挿したときは、書き込み後も書き込みモードのまま止まるので、挿し直す。

S3 は 3 秒後に USB ホストになり、USB シリアル/JTAG が消える。

BOOT を使う場合（ファームが壊れて 3 秒の窓がないときなど）:

1. 5V 給電（CH9102F の USB）とキーボードを S3 から外す
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
