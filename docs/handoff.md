# 引き継ぎメモ（2026-10-13 夜 → クラウドセッション）

ローカルのセッション（実機あり）から、クラウドのセッション（実機なし）にソフトウェアの改善を引き継ぐためのメモ。
全体像は [README.md](../README.md)・[ROADMAP.md](../ROADMAP.md)・[docs/firmware.md](firmware.md) を先に読むこと。

## クラウドセッションの成果（2026-10-13 夜）

ブランチ `claude/vibrant-ramanujan-m09xcb`（1 コミット 1 件。良いものだけ cherry-pick できる）。
**ファームは Xtensa のツールチェーンが入れられず（GitHub からのダウンロードがクラウドでは遮断されている）、ビルド未確認。**
ホスト側は `cargo test --workspace`（79 件）と clippy が通る。既定のビルド（feature なし）の挙動は変えていない。

| # | コミット | 内容 | 確かめたこと | 未確認 |
|---|---|---|---|---|
| 1 | Pointer: extract relative-mode conversion | 相対モードの変換を `crates/hikyaku-pointer/src/relative.rs`（`Relay`）に切り出し。ファームはそれを呼ぶだけに | 元のファームのコードの写し（テスト内の `Golden`）と、実機ログ全体 + 乱数パケット（スクロールモード含む）で **出力が 1 ビットも違わない** | ファームのビルド |
| 2 | `trackpad-still-filter` | 静止中のぶれを抑える（位置の不感帯 + ヒステリシス。[詳細](#静止フィルタの考え方)） | 置いたままのぶれが出ない／スッと触ったときの最初のパケットがそのまま通る／ゆっくりの動きで距離が減らない、など 7 件 | 実機の体感 |
| 3 | `trackpad-soft-tap` | パッド側のタップを切り、パケットの時間でタップを判定（ZIdle=5 を「離した」合図に）。[詳細](#自前のタップ判定の考え方) | タップ→クリック、スワイプ→クリックしない、置いたまま→クリックしない、ダブルタップ、など 6 件 | **相対モードで ZIdle=5 にしたときにゼロパケットが本当に来るか**（来なければクリックは一切出ない。安全側） |
| 4 | `trackpad-sens-1x/2x/3x` | ADC 減衰を feature で選ぶ。既定 4X のまま | — | 実機 |
| 5 | `usb-trace` + docs | USB のキーレポートをログに出す feature。OTA の仮説（[ota.md](ota.md)）。整理の提案（下） | — | — |

試し方（1 つずつ）: `FEATURES="trackpad trackpad-selftest trackpad-trace trackpad-still-filter" tools/hk flash-log` のように足す。
ログの再生: `cargo run -q -p hikyaku-pointer --example replay_rel -- <log> quiet=1 skip_fc=1 [still=1] [tap=1]`。

### 静止フィルタの考え方

ぶれは、パッドが報告する**位置**が ±1〜2 揺れているもので、移動量を足すと増えない（行って戻る）。
「静止」状態では移動量を足し込むだけにして、合計が 3 に届いたら溜めた分をまとめて出す（動きは失わない）。1 パケットで 3 以上
動いたら溜めずにそのまま通す（スッと触ったときの反応を鈍らせない）。一度動き出したら「移動」状態で 1:1（ゆっくり動かしても刻まない）。
直近 8 パケット（約 80ms）の合計が 3 未満になったら静止に戻す。100ms 途切れたら（指を離した）溜めた分は捨てる。
しきい値は `StillFilter::DEFAULT`（`threshold: 3, gap_ms: 100`）。

### 自前のタップ判定の考え方

相対モードのパケットには指の有無がないので、パケットが 30ms 以上途切れたら接触の終わりとみなす。接触が 200ms 以内、
移動量の合計（|dx|+|dy|）が 10 以内なら**タップ**として左クリックを 30ms 押す。スワイプ（移動量が大きい）や長押しはクリックしない。
「置いたまま止めている間はパケットが来ない」場合に置いた直後の静けさをタップと誤らないよう、**接触の終わりにゼロパケット
（dx=dy=0）を要求する**。これはパッドの ZIdle（離したときに送る空パケットの数。QMK は 5）で出るので、この feature では ZIdle=5 にする
（`RelativeOptions::z_idle`。既定は 0 のまま）。クリックは離してからおよそ 50ms + 30ms 後に出る。
設定は `SoftTap::DEFAULT`。もし「パッド側のタップを切ると動きの反応まで悪くなる」が再現するなら、`primary_tap: true` のまま
パケットのボタンビットを無視する（`Relay` はそうしている）変更が 1 行で試せる（`trackpad/mod.rs` の `REL_OPTIONS`）。

### 整理の提案（消すのは持ち主の判断。ここでは提案だけ）

- **絶対座標の経路を消す** → **2026-10-13 に削除済み**（持ち主の判断）。`trackpad-absolute` / `trackpad-lowbyte` / `trackpad-sweep`、
  `hikyaku-pointer` の `Pointer`・`tracker`・`examples/replay.rs`、`hikyaku-pinnacle` の `init` / `read_absolute(_checked)` / `parse_absolute` / `Checked`。
  消す前の状態はタグ `absolute-mode-last`（`main` の 82ea5c0）に残っている。`trackpad-selftest`・`trackpad-trace` は残した。
  `hikyaku-pointer` は `relative` の置き場として残した（クレート名の変更はしていない）。
- **ビルドの警告**: 実機の PC でビルドして確認した（2026-10-13）。削除後に残るのは `main.rs` の 2 件だけ（不要な `unsafe` ブロック、
  読まれない `battery` フィールド）。どちらもこの PR より前からある。
- `SCROLL_MODE` は誰も true にしていない（Phase 5 の「キー押下中はスクロール」用）。使うときは `usb_host` のキーレポートで特定キーを見て立てる。

## 作業のルール（必ず守る）

1. **`main` に直接 commit しない。** ブランチ（例: `cloud/2026-10-13-trackpad`）を切って push し、PR にする。
   翌朝、持ち主が実機で 1 つずつ試して、良かったものだけ取り込む。
2. **今の既定の挙動を変えない。** 下の「今の既定の設定」は、持ち主が実機で「調子が良い」と確認した状態。
   新しい処理（フィルタ・自前のタップ判定・感度の変更など）は、**Cargo の feature の裏**に置き、既定ではオフにする。
   既定の値を変えたい提案は、コードを変えずに PR の説明に書く。
3. **1 つの PR・1 つの feature で、1 つのことだけ変える。** 過去に「速度だけ変えて」と頼まれたのに他も変えて、悪化させたことがある。
4. 実機での体感は確認できない。代わりに、ホストで動くテスト（`cargo test --workspace`）とログの再生で確かめ、
   **何を確かめて、何が未確認か**を PR に書く。
5. 秘密のものはリポジトリに入れない（`~/.config/hikyaku/ota.key` など。クラウドには元からない）。
   Cirque の仕様書 PDF も入れない（配布不可）。

## 環境

- ホスト側のテスト: リポジトリ直下で `cargo test --workspace`（2026-10-13 時点で全部通る。`hikyaku-pinnacle` 21 件、`hikyaku-pointer` 34 件など）。
- ファーム（`firmware/hikyaku-s3`）は Xtensa 向け。ビルドには `espup install` → `. ~/export-esp.sh` が必要
  （`espup` は GitHub API を使うので、`GITHUB_TOKEN` がないとレート制限の 403 で失敗することがある）。
  ビルドは `tools/hk build`、feature は `FEATURES="trackpad trackpad-trace" tools/hk build` のように渡す。
  クラウドで Xtensa の環境が作れなければ、ファームの変更は「ビルド未確認」と PR に明記する。
- 書き込み（`tools/hk flash`）とログ（`tools/hk log`）は実機が要るので、クラウドでは使えない。

## 今の状態

- USB キーボード（7skb）→ ESP32-S3 → BLE HID → Mac は**動いている**（キー・メディアキー・ボンディング保存）。
- トラックパッド（Cirque TM035035 / Pinnacle 1CA027、SPI）は**相対モードで動いている**。円も描ける。ただし:
  - 配線がメス-メスのジャンパー線で、接触が不安定。持ち主がユニバーサル基板にはんだ付けする予定（[hardware/assembly.md](hardware/assembly.md)）。
    **同じファームでも日によって調子が変わるのは、まず配線を疑う。**
  - 軽く素早く触ったときの反応がまだ弱い。
  - 指を置いて止めているだけでも、カーソルが少しぶれる。
  - スワイプ中にタップ（クリック）が誤って出ることがある。
- BLE 経由の OTA は実装済みだが、**実機では成立していない**（[ota.md](ota.md) の「状況」）。今は有線で書き込んでいる。

### 今の既定の設定（`firmware/hikyaku-s3/src/bin/trackpad/mod.rs`）

| 項目 | 値 | 経緯 |
|---|---|---|
| モード | 相対モード（パッドが計算した移動量をそのまま使う） | 絶対座標から自前で移動量を作る方式は使い物にならなかった。パッドは丸いので、画面の座標と対応させる方式は採らない（持ち主の方針） |
| SPI | モード 1、**2MHz** | 1MHz 以下では上位ビットが化ける（読み戻しテストで確認）。4MHz では Y が抜けたり、動きが飛んだりした |
| ADC 減衰（タッチ感度） | `Attenuation::X4`（いちばん鈍い） | 2X / 1X は**未試行**。軽いタッチへの反応を上げる候補 |
| パッド側のタップ | 有効（`primary_tap: true`）→ 左クリック | パッド側のタップを切ると、動きの反応まで悪くなった（2026-10-13、取り消し済み） |
| パッド側の軸の反転 | **使わない** | Y 反転を入れると、相対モードの Y がほとんど出なくなった |
| 向き | `SWAP_XY = true`、`SCREEN_X_INVERT = true`、`SCREEN_Y_INVERT = true` | パッドが 90 度回った向きで付いている。実機で確認済み |
| 倍率 | `REL_GAIN = 2` | 1:1（ZMK・Cirque 公式）では遅く、4 倍（QMK の既定）では速すぎた |
| 読み取りの間隔 | 5ms ごとにポーリング（DR ピンは配線済みだが未使用） | パッドは約 100Hz で出す |
| 配線ミスの検出 | 起動時に firmware id が `0x07` でなければトラックパッドを止める | 配線が外れていると `fc fc fc` のゴミが来て、カーソルが暴れた |

持ち主が「調子が良い」と言っているビルドは、`FEATURES="trackpad trackpad-selftest trackpad-trace"`。
feature を外したビルドの方が悪く感じた、という報告がある（原因不明。配線のばらつきの可能性が高いが、未確認）。

## やってほしいこと（優先順）

（2026-10-13 夜のクラウドセッションで 1〜5 は着手済み。上の「成果」を参照。残りは実機での確認と、6 の実機検証）

1. **ログの再生でテストできる仕組み**
   `tools/traces/` の実機ログ（`rel <ms> raw=[...]` の行）を読み、相対モードの処理（軸の入れ替え・反転・倍率・持ち越し・レポートの分割）を
   ホストで再生して、出てくるマウスレポートを確かめられるようにする。
   そのために、`trackpad/mod.rs` の変換部分を、ホストでテストできる純粋な関数（`crates/` 側）に切り出す。**出力は今と 1 ビットも変えない**ことをテストで示す。
   ログの注意点は [tools/traces/README.md](../tools/traces/README.md)（ほとんどが配線不良の時期のもの）。
2. **静止中のぶれを抑えるフィルタ**（feature の裏）
   指を止めているときの ±1〜2 の揺れを抑える（例: 小さい動きの不感帯、短い窓での平均、ヒステリシス）。
   動き始めの反応を鈍らせないことが条件（持ち主は「スッと触ったときの反応」を重視している）。
3. **自前のタップ判定**（feature の裏、パッド側のタップと切り替え）
   スワイプ中の誤クリックを減らす。相対モードのパケットには指の有無が直接ないので、パケットが来ている／来ていない時間と移動量で判定する案。
4. **タッチ感度（ADC 減衰）を feature で選べるようにする**（例: `trackpad-sens-2x`、`trackpad-sens-1x`）。既定は 4X のまま。
5. **整理**: 使っていない絶対座標の経路（`trackpad-absolute`、`trackpad-lowbyte`、`trackpad-sweep`）と `hikyaku-pointer` の扱いを、
   消すか残すか提案する（**消すのは PR で提案するだけ**にして、持ち主の判断を待つ）。ビルドの警告を減らす。
6. （余裕があれば）OTA が実機で成立しない原因をコードから調べ、[ota.md](ota.md) に仮説を書く。

## 参考にした実装

- QMK `drivers/sensors/cirque_pinnacle.c`（相対モードは 4 倍がけが既定）
- Zephyr `drivers/input/input_pinnacle.c`（ZMK が使う。初期化の手順はこれに合わせた。移動量は 1:1）
- Cirque 公式 `SPI_FlatOverlay_USBMouse.ino`

パケットの形式や RAP（レジスタの読み書き）は `crates/hikyaku-pinnacle/src/lib.rs` のコメントと [hardware/trackpad-tm035035.md](hardware/trackpad-tm035035.md) を参照。
