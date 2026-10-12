#!/bin/bash
# CH9102F（UART→USB 変換）が PC に現れたら、S3 のログを一定時間キャプチャする。
#
#   tools/scripts/capture_when_ready.sh <秒数> <出力ファイル>
#
# S3 を元の構成（キーボード + CH9102F）に戻す前に起動しておくと、挿した瞬間から取れる。
# ポートを開いても S3 はリセットされない（CH9102F の DTR/RTS は S3 につないでいないため）。
set -u
DUR="${1:-120}"
OUT="${2:-/tmp/s3.log}"
cd "$(dirname "$0")/../.."
# CH9102F（ベンダー ID 1a86）のポートを探す（S3 と両方挿していても取り違えない）
find_port() {
  local p
  for p in /dev/ttyACM* /dev/ttyUSB*; do
    [ -e "$p" ] || continue
    udevadm info -q property -n "$p" 2>/dev/null | grep -q "ID_VENDOR_ID=1a86" && { echo "$p"; return; }
  done
}
for _ in $(seq 1 6000); do
  port="$(find_port)"
  if [ -n "$port" ]; then
    sleep 0.5
    exec python3 tools/scripts/uart_cap.py "$DUR" "$port" > "$OUT" 2>&1
  fi
  sleep 0.1
done
