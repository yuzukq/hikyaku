# 実機ログ（トラックパッド）

CH9102F 経由で取った S3 のログ（`tools/hk log`）。色コードと、接続相手のアドレス行は取り除いてある。

| ファイル | 中身 | 注意 |
|---|---|---|
| `2026-10-13-relative-all.log` | 相対モード（`trackpad-trace`）の一晩分。`rel <ms> raw=[b0, b1, b2] dx= dy=` が 1 パケット 1 行。`selftest mode=...` の読み戻しテストの結果も含む | **ほとんどがメス-メスのジャンパー線で接触が不安定だった時期のもの。** `raw=[fc, fc, fc]`（dx=-4 dy=-4）は配線が外れているときのゴミ。`firmware id=0xfc` や `0x00` で始まった起動の区間は信用しない。正常に起動した区間（`firmware id=0x07`、末尾）は、ほぼ動きがない |
| `2026-10-13-absolute-sweep.log` | 旧方式（絶対座標）の `tp <ms> <x> <y> <z> khz=<SPI周波数> raw=[...]`。SPI の周波数を切り替えながら取ったもの | 1MHz 以下では上位ビットが化ける（2MHz 以上が正しい） |

起動の区切りは `ESP-ROM` / `rst:0x` の行。区間ごとの内訳（正常なパケット / `fc` のゴミ）は次のコマンドで出せる。

```sh
awk '/ESP-ROM|rst:0x/{b++} /rel [0-9]+ raw=/{ if ($0 ~ /fc, fc, fc/) fc[b]++; else ok[b]++ }
     END { for (i in ok) print "boot", i, "ok", ok[i], "fc", fc[i] }' tools/traces/2026-10-13-relative-all.log
```

**きれいな配線での動きのログは、まだない。** 基板化（[docs/hardware/assembly.md](../../docs/hardware/assembly.md)）の後に、
`FEATURES="trackpad trackpad-selftest trackpad-trace" tools/hk flash` → `tools/hk log` で取り直す予定。
