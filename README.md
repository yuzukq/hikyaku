# hikyaku（飛脚）

手持ちの USB キーボードの入力を受け取り、Bluetooth (BLE) で Mac に届ける無線中継器。
トラックパッドも内蔵し、分割キーボードの真ん中に置いて使うことを想定している。

```
[USB キーボード] --USB--> [hikyaku: XIAO ESP32S3 + TM035035] --BLE HID--> [Mac]
                                   ↑
                             トラックパッド
```

## 構成

| 役割 | 部品 | メモ |
|---|---|---|
| マイコン | Seeed XIAO ESP32-S3 | USB OTG でホストになれる + BLE。1チップで完結 |
| トラックパッド | Cirque Pinnacle TM035035（SPI 版） | 35mm 円形。J1 は 12 ピン 0.5mm FFC |
| USB 接続 | USB-C ⇔ USB-C ケーブル | XIAO の USB-C をホストポートとして使う |
| 電源 | モバイルバッテリー（Type-C）+ 秋月 Type-C DIP 化キット（CC に 5.1kΩ ×2） | S3 の 5V ピンへ給電。キーボードにも供給される |
| ログ | 秋月 AE-CH9102F-TYPEC-BO | ホスト動作中は USB シリアルが使えないため UART で取る |

部品の詳細と手持ちの状況は [docs/hardware/parts.md](docs/hardware/parts.md)。

## 設計上の決定

- **マイコンは ESP32-S3**：nRF52840 は USB ホスト非対応のため不採用。ESP32-C3 も USB ホスト不可（Serial/JTAG のデバイス側のみ）で不採用。BLE 検証には使った
- **言語は Rust**（esp-hal + embassy + trouble）。USB ホストは esp-hal の OTG ホストドライバ + `embassy-usb-host`
- **一次電池は使わない**：USB キーボードへの 5V 給電を含めると全体で約 100mA 前後になり、コイン電池では動かず、乾電池では交換頻度が高い
- **ESP32-S3 の BLE は macOS で安定して動作した**（ボンディング保存・再起動後の自動再接続も確認済み）。nRF52840 は不要
- **HID レポートは詰め替える**：受け取ったキーボードのレポート記述子をそのまま流さず、固定の記述子（キーボード + コンシューマ + マウス）に変換する。キーボードごとに記述子が変わると macOS のペアリングキャッシュで不具合が出るため

## 既知の制約

- XIAO ESP32S3 の 5V ピンは USB-C の VBUS に直結している。ホスト動作中はここに外部から 5V を入れてキーボードに給電する
- 電池駆動では XIAO 単体で VBUS に 5V は出ない。昇圧モジュールが必要
- USB-C をホストに使うと USB CDC のシリアルログが使えない。デバッグは UART（USB-シリアル変換）で行う

## 現状

7skb（USB）→ S3（USB ホスト）→ BLE → Mac のパススルーが動作する（通常キー・メディアキー・マウス系、ボンディング保存つき）。次はトラックパッドの統合。

- 進捗と今後の計画: [ROADMAP.md](ROADMAP.md)
- 資料（配線・電源・仕様メモ・ビルド/書き込み手順・落とし穴）: [docs/](docs/README.md)
