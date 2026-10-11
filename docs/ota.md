# OTA（BLE 経由のファーム更新）

> **状況（2026-10-12）: 実機ではまだ成立していない。** 実装とビルドは済んでいる。通常の更新は有線（BOOT ボタン）で行っている。
> これまでに分かったこと:
> - 購読（CCCD）の枠が 1 つだと、Mac が枠を使い切って PC が通知を購読できない → `connections_max = 4` に修正済み
> - 接続待ちを途中で止めて OTA に切り替える方式は、PC の接続が成立しなかった → 再起動して起動直後に OTA モードに入る方式に変更済み（RTC フラグ）
> - その版を有線で書き込んだ後の試験では、組み合わせのキーを押しても S3 のログに `usb: OTA chord` が出ず、OTA モードに入れなかった。原因は未調査（キーボードが S3 に認識されていなかった可能性）
> - アップロード側のログは `python -u` でバッファなしにしてある
> 再挑戦するときは、まず S3 のログで `usb: keyboard ready` と `usb: OTA chord` が出ることを確認する。

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
