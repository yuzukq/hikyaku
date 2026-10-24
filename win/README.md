# hikyaku の Windows アプリ（C# / WPF）

ショートカットリングの Windows 版。Mac 版（`mac/HikyakuRing`）と同じことをする: タスクトレイに常駐し、hikyaku の F23・本体のボタンのダブルタップ・
Ctrl+Alt+R でリングを出す。仕様は [docs/design/ring.md](../docs/design/ring.md)、2 台の切り替えは [docs/design/multi-host.md](../docs/design/multi-host.md)。

| フォルダ | 中身 |
|---|---|
| `RingCore` | 通信の形式・リングの判定（`RingMachine`）・設定・キーの対応。Windows に依存しない |
| `RingCore.Tests` | そのテスト（Mac 版のテストの移植。Linux でも動く） |
| `RingProbe` | 試作: Windows が、HID として接続中の hikyaku の独自サービスをアプリに開かせるかの確認 |
| `HikyakuRing` | アプリ本体（トレイ・リング・ノブ・設定画面・音量 / 明るさ / シーク） |

## ビルド

```sh
win/build.sh        # Linux / Mac（Docker の .NET 8 SDK）。テスト → win/out/HikyakuRing.exe・RingProbe.exe・hikyaku-windows.zip
win/build.sh test   # テストだけ
```

exe は .NET のランタイムを同梱した単一ファイル（Windows に .NET を入れなくてよい）。Windows 上でビルドするなら .NET 8 SDK を入れて
`dotnet test RingCore.Tests`、`dotnet publish HikyakuRing -c Release -r win-x64 -o out`。

## Windows での試し方

1. hikyaku を 2 台目（`hikyaku2`）に切り替える（7sPro の F22、または本体のボタンをパッドに触れずに 3 秒押し続ける。LED が青く 2 回点滅）
2. Windows の設定 → Bluetooth とデバイス → デバイスの追加 → Bluetooth → `hikyaku2` を選んでペアリング。キー・マウス・スクロールが動くか確かめる
3. `RingProbe.exe` をコマンドプロンプトで実行する。「購読: Success」「設定: …」が出て、F23 や「o + Enter」でリングを開くと、
   指の角度が流れてくれば成功（Windows でも独自サービスを開ける）
4. `HikyakuRing.exe` を実行する（タスクトレイにリングのアイコンが出る。ダブルクリックで設定画面）

ログ: `%LOCALAPPDATA%\HikyakuRing\log.txt`。設定: `%APPDATA%\HikyakuRing\config.json`。

## Windows で違うところ（Mac 版と比べて）

- アイコンは SF Symbols の代わりに Segoe Fluent Icons の文字（設定画面で `E767` のような 16 進の文字コードで指定する）
- 明るさは内蔵画面だけ（WMI）。外部モニタ（DDC/CI）は未対応
- シークは Windows のメディア操作（GSMTC）。アプリが位置の変更に応じない場合は動かない（ログに出る）
- 音量キー 1 回で 2%（Mac は 1/16）
- 「送り先」タブで、1 台目 / 2 台目の切り替えと、送り先ごとのスクロールの向き（Windows の既定は反転しない）を変えられる
