#!/bin/bash
# S3 が PC に現れたら、すぐ有線で書き込む（BOOT を押しながら挿す操作の、タイミングを気にしなくてよくする）。
#
#   tools/scripts/flash_when_ready.sh [ビルド済み ELF（省略時は firmware/hikyaku-s3 の release）]
#
# 失敗したら、ポートが現れ直すのを待って再試行する（最大 5 分）。パーティション表（OTA 用）も書く。
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/../../firmware/hikyaku-s3" || exit 1
ELF="${1:-target/xtensa-esp32s3-none-elf/release/hikyaku-s3}"
end=$((SECONDS+300))
while [ $SECONDS -lt $end ]; do
  if [ -e /dev/ttyACM0 ]; then
    echo "$(date +%T) port present; flashing"
    if timeout 40 espflash flash --port /dev/ttyACM0 --partition-table partitions.csv "$ELF" 2>&1 | tail -4; then
      [ "${PIPESTATUS[0]}" -eq 0 ] && { echo "$(date +%T) FLASH OK"; exit 0; }
    fi
    echo "$(date +%T) flash failed; waiting"
    sleep 1
  fi
  sleep 0.05
done
echo timeout
exit 1
