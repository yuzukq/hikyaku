# OTA（BLE 経由のファーム更新）

> **状況（2026-10-12）: 実機ではまだ成立していない。** 実装とビルドは済んでいる。通常の更新は有線（BOOT ボタン）で行っている。
> これまでに分かったこと:
> - 購読（CCCD）の枠が 1 つだと、Mac が枠を使い切って PC が通知を購読できない → `connections_max = 4` に修正済み
> - 接続待ちを途中で止めて OTA に切り替える方式は、PC の接続が成立しなかった → 再起動して起動直後に OTA モードに入る方式に変更済み（RTC フラグ）
> - その版を有線で書き込んだ後の試験では、組み合わせのキーを押しても S3 のログに `usb: OTA chord` が出ず、OTA モードに入れなかった。原因は未調査（キーボードが S3 に認識されていなかった可能性）
> - アップロード側のログは `python -u` でバッファなしにしてある
> 再挑戦するときは、まず S3 のログで `usb: keyboard ready` と `usb: OTA chord` が出ることを確認する。
> コードを読んで立てた仮説は下の「[実機で成立しない原因の仮説](#実機で成立しない原因の仮説2026-10-13-クラウドセッションコードを読んだだけ)」。

有線の抜き差し・BOOT ボタンなしで、PC から BLE でファームを書き換える。
実装: `firmware/hikyaku-s3/src/bin/ota/`（受信側）、`tools/ota/`（PC 側）。

## 使い方

```fish
tools/ota/ota.sh init       # 初回だけ: 鍵の生成（~/.config/hikyaku/ota.key）と Python 環境（bleak）
tools/ota/ota.sh flash-usb  # 最初の 1 回だけ有線で: OTA 用のパーティション表も書く（BOOT ボタン操作が必要）

# 以降は毎回これだけ:
tools/ota/ota.sh build      # 鍵を埋め込んでビルド → イメージ（.bin）作成
# キーボードで Ctrl+Alt+Shift+Gui+Enter+U を押す（OTA モード。ホストとの接続が切れ、hikyaku-OTA として待つ）
tools/ota/ota.sh upload     # BLE で送信 → 検証 → 再起動
```

`tools/ota/ota.sh`（引数なし）は build + upload。ビルド時の機能は `FEATURES="trackpad trackpad-trace" tools/ota/ota.sh` のように指定する（既定は `trackpad`）。

OTA モードは 180 秒で自動的に通常に戻る（再起動）。アップロード中は Mac との接続が切れ、再起動後に自動で戻る。

**OTA モードで接続を受けるのは、`ota.sh init` で記録した PC（BLE アドレス）だけ**（`~/.config/hikyaku/ota.peer`、ビルド時に `HIKYAKU_OTA_PEER` として埋め込む）。
Mac のように同じアドレスで自動再接続してくる相手を弾くため。このアドレスを入れていない（古い）ファームに OTA するときだけは、**Mac の Bluetooth を切って**から OTA モードに入る。

## 仕組み

- **OTA モードへの入り方:** キーボードの組み合わせを検出すると、RTC メモリにフラグを立てて**再起動**し、起動直後に OTA モードに入る。
  （最初は、接続待ちの処理を途中で止めて切り替えていたが、PC の接続が成立しなかったため、BLE の状態が新しい起動直後に入る方式にした。）
- **購読状態の枠:** `#[gatt_server(connections_max = 4)]`。既定の 1 だと、ボンディング済みの Mac が枠を使い切り、別の相手（PC）が通知を購読できない。

- **パーティション表**（`firmware/hikyaku-s3/partitions.csv`）: `nvs`（0x9000、ボンディングの保存先）/ `otadata` / `phy_init` / `ota_0` / `ota_1`（各 1.875MB）。
  現在動いていない方のスロットへ書き、検証後に `otadata` を切り替えて再起動する。
- **起動時**に、動いているイメージを「有効」にマークする（`mark_app_valid`）。
- **GATT サービス**（UUID `6b9a0001-6f1e-4b8e-9d57-7f0d0a5b0001`）: `ctrl`（`…0002`, 書き込み/通知）と `data`（`…0003`, write-without-response）。
  **OTA モード以外では書き込みを拒否する。** HID サービスの後ろに追加してあり、既存のハンドルは動いていない。
- **プロトコル**: `HELLO`（ナンスを受け取る）→ `START`（サイズ + SHA-256 + HMAC）→ データ（240 バイト単位、16KiB ごとに進捗通知でフロー制御）→ `FINISH`。
  詳細は `ota/mod.rs` の冒頭コメント。

## 守り

- キーボードの組み合わせ（物理操作）で入る OTA モードだけが書き込みを受け付ける + 180 秒の時間制限。
- 共有鍵による HMAC 認証（HMAC-SHA256(key, ナンス ‖ サイズ ‖ SHA-256)）。ナンスは 1 回限り。
- SHA-256 とイメージのヘッダー（0xE9）の確認に通らないと切り替えない。
- 鍵は `~/.config/hikyaku/ota.key`（リポジトリには入れない）。ビルド時の環境変数 `HIKYAKU_OTA_KEY` としてファームに埋め込まれる。
  **鍵なしでビルドすると OTA は無効**（OTA モードに入っても何もしない）。鍵を変えたら、有線で 1 回書き直す。
- ロールバックは無い。壊れたファームを書いてしまったら、従来どおり BOOT を押しながら挿して有線で書き直す。

## 注意・既知のこと

- OTA 中は USB キーボードのレポートが取りこぼされる（フラッシュの消去でしばらく止まる）。
- Windows/Linux など複数の相手と使っているときは、OTA モードが入ると現在のホストとの接続は切れる。
- BlueZ（Linux）は GATT をキャッシュするので、`ota.sh upload` は先に `bluetoothctl remove` している。
- 有線書き込み（`flash-usb`）でパーティション表を変えると、`app` の置き場が `0x10000` から `0x20000`（ota_0）に変わる。`nvs`（ボンディング）は同じ場所なので消えない。

## 実機で成立しない原因の仮説（2026-10-13, クラウドセッション。コードを読んだだけ）

実機は無いので、段階ごとに「ログで何を見るか」と「怪しい点」を挙げる。上から順に潰す。

### 段階 A: 組み合わせキーが検出されない（`usb: OTA chord` が出ない）

検出は `usb_host` の `KeyStatusUpdate` 1 つごとに `is_ota_chord` で行う。条件は **同じレポートの中に** Ctrl・Shift・Alt・Gui
（左右どちらでも）の 4 種の修飾キーが全部立ち、かつキー配列に Enter(0x28) と U(0x18) が両方あること。

1. **キーボードが S3 に認識されていなかった**（最有力。ログに `usb: keyboard ready` が無い）。S3 は起動 3 秒後に USB ホストになる。
   有線で書き込んだ直後の構成戻し（5V・キーボードの挿し直し）で、列挙に失敗していると `usb: enumerate failed` / `usb: not a boot keyboard` が出る。
2. **キーマップの都合で、6 つが同時に 1 レポートに乗っていない。** QMK の mod-tap（例: Enter を押しっぱなしで修飾キーになる設定）だと、
   Enter を押し続けている間は 0x28 ではなく修飾キーが送られる。また Gui を OS 側で無効にしていても、USB 上は送られるので問題ない。
   → `usb-trace` feature（`FEATURES="trackpad usb-trace" tools/hk flash`）で、押したときのレポート（`usb: key report mods=.. keys=..`）を
   そのまま見られるようにした。`mods=0xXX` の下位/上位 4 ビットに 1・2・4・8 のどれかずつが立ち、`keys` に `0x28` と `0x18` があれば検出されるはず。
3. 7skb が **NKRO** で送っている可能性は低い（`KbdHandler` はブートプロトコル。IF0 は 6KRO で動作確認済み）。

### 段階 B: 検出はされたが OTA モードで起動しない（`ota: rebooting into OTA mode` の後に `ota: mode on` が出ない）

1. **RTC メモリのフラグ**（`OTA_BOOT_FLAG`、`rtc_fast` + `persistent`）は `software_reset()` をまたいで残る前提。
   もし `ota: mode on` が出ず `advertising as hikyaku` に戻るなら、フラグが消えている。確認は、起動直後に `take_ota_boot_flag` の
   読み取り値をログに出す（1 行追加）。消えていれば、フラグを NVS（`bond_store` と同じ 0x9000 の領域の別セクタ）に置く方式に変える。
2. `ota: HIKYAKU_OTA_KEY was not set at build time` が出たら、鍵が埋め込まれていない。`tools/hk build` と `tools/ota/ota.sh build` は
   どちらも `~/.config/hikyaku/ota.key` を読むが、**ファイルが無いと `hk` は黙って鍵なしでビルドする**（OTA が無効なファームになる）。

### 段階 C: OTA モードに入ったが PC がつながらない（`ota: connected` が出ない）

1. **接続を受ける相手の絞り込み**（`HIKYAKU_OTA_PEER` → フィルター受け入れリスト）は、PC のアドレスを **PUBLIC** として登録する。
   `ota.sh init` が記録するのは `bluetoothctl list` のコントローラのアドレス（public）なので通常は一致するが、BlueZ の privacy 設定
   （`/etc/bluetooth/main.conf` の `Privacy = device` など）が有効だと PC は **ランダムアドレス**で接続しに来て、弾かれる。
   切り分け: `ota.peer` を空にしてビルドすると `ota: HIKYAKU_OTA_PEER not set; accepting any central` になり、誰でも受ける
   （このときは Mac の Bluetooth を切る。トラブルホストの接続枠 `CONNECTIONS_MAX = 1` を Mac が先に使ってしまうため）。
2. OTA モードでも **アドレスは通常時と同じ** `FF:E4:05:1A:8F:FF`。ボンディング済みの Mac は広告を見るとすぐ再接続しに来る。
   受け入れリストがあれば弾かれるが、Mac 側に「接続中」の表示が出たり消えたりするのは正常。
3. `upload.py` は広告の名前 `hikyaku-OTA` で探す。BlueZ は同じアドレスの名前 `hikyaku` をキャッシュしているので、
   `ota.sh upload` が先に `bluetoothctl remove` している。手で `upload.py` を叩くときも先に remove する。

### 段階 D: つながったが HELLO の応答（通知 `81`）が来ない

1. 通知は PC が CCCD を書いた（購読した）後でないと届かない（trouble の `notify` は未購読でも成功を返す）。`gatt write` のログで
   CCCD（`ctrl` の cccd_handle）への書き込みがあるか見る。`connections_max = 4` は済み。
2. `ctrl` キャラクタリスティックは固定長 80 バイト。`HELLO` は 1 バイトの書き込みなので、trouble が「長さが違う」と拒否していないか
   （`gatt error` のログ）。拒否されるなら、PC 側で 80 バイトに 0 埋めして送るのが手っ取り早い。

### 段階 E: データ送信中に切れる・止まる

1. **フラッシュの消去・書き込み中は割り込みを止める**（`esp-storage`）。消去はセクタ（4KB）ごとに数十 ms かかり、その間 BLE の
   コントローラ（`esp-radio`）がイベントを取りこぼす。`data()` は必要になったセクタをその場で消すので、16KiB ごとの進捗通知の前後で
   接続が切れる（`ota: disconnected`）なら、これ。対策の候補: START の時点で領域をまとめて消す（1.875MB で数秒。PC 側の
   `expect(0x82)` のタイムアウト 15 秒は足りるはず）→ データ受信中は書き込みだけにする。
2. 切れたら PC 側は `BleakError` で落ち、S3 は `ota: disconnected, waiting again` で待ち続ける（180 秒）。やり直すには HELLO からになる
   （ナンスは 1 回限りなので、同じ START は通らない。これは仕様どおり）。

### まとめ（次にやること）

1. `FEATURES="trackpad usb-trace" tools/hk flash-log` で、組み合わせキーを押したときのレポートを見る（段階 A）。
2. `usb: OTA chord` → `ota: rebooting into OTA mode` → `ota: mode on for 180 s` の 3 行がそろうか。
3. そろえば、`ota.sh upload` の出力と S3 の `ota: connected` / `gatt` のログを突き合わせる（段階 C・D）。
