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
for _ in $(seq 1 6000); do
  if [ -e /dev/ttyACM0 ] && lsusb | grep -q "1a86"; then
    sleep 0.5
    exec python3 tools/scripts/uart_cap.py "$DUR" /dev/ttyACM0 > "$OUT" 2>&1
  fi
  sleep 0.1
done
