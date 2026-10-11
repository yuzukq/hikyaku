#!/bin/bash
# hikyaku ファームの OTA 更新ツール。
#
#   tools/ota/ota.sh init         鍵の生成と Python 環境（bleak）の用意（初回だけ）
#   tools/ota/ota.sh build        鍵を埋め込んでビルドし、書き込み用イメージ（.bin）を作る
#   tools/ota/ota.sh flash-usb    有線で書き込む（OTA 用のパーティション表も書く。最初の 1 回は必須）
#   tools/ota/ota.sh upload       BLE で書き込む（事前にキーボードで OTA モードにする）
#   tools/ota/ota.sh              build + upload
#
# 環境変数: FEATURES（既定 trackpad）。例: FEATURES="trackpad trackpad-trace" tools/ota/ota.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
FW="$ROOT/firmware/hikyaku-s3"
KEYFILE="${HIKYAKU_OTA_KEYFILE:-$HOME/.config/hikyaku/ota.key}"
VENV="$ROOT/tools/ota/.venv"
FEATURES="${FEATURES:-trackpad}"
ELF="$FW/target/xtensa-esp32s3-none-elf/release/hikyaku-s3"
BIN="$FW/target/hikyaku-s3.ota.bin"
ADDR="FF:E4:05:1A:8F:FF"

need_env() {
  [ -f "$HOME/export-esp.sh" ] && . "$HOME/export-esp.sh"
  export PATH="$HOME/.cargo/bin:$PATH"
}

cmd_init() {
  mkdir -p "$(dirname "$KEYFILE")"
  if [ ! -f "$KEYFILE" ]; then
    python3 -c 'import secrets; print(secrets.token_hex(32))' > "$KEYFILE"
    chmod 600 "$KEYFILE"
    echo "generated key: $KEYFILE"
  else
    echo "key exists: $KEYFILE"
  fi
  [ -d "$VENV" ] || python3 -m venv "$VENV"
  "$VENV/bin/pip" install -q bleak
  echo "ready"
}

cmd_build() {
  need_env
  [ -f "$KEYFILE" ] || { echo "no key. run: $0 init"; exit 1; }
  ( cd "$FW" && HIKYAKU_OTA_KEY="$(cat "$KEYFILE")" cargo build --release --features "$FEATURES" )
  espflash save-image --chip esp32s3 "$ELF" "$BIN"
  echo "image: $BIN ($(stat -c %s "$BIN") bytes)"
}

cmd_flash_usb() {
  need_env
  ( cd "$FW" && espflash flash --partition-table partitions.csv "$ELF" )
}

cmd_upload() {
  [ -f "$BIN" ] || { echo "no image. run: $0 build"; exit 1; }
  # BlueZ が古い GATT キャッシュを持っていると新しいサービスが見えないので、先に忘れさせる。
  bluetoothctl remove "$ADDR" >/dev/null 2>&1 || true
  "$VENV/bin/python" "$ROOT/tools/ota/upload.py" "$BIN" "$(cat "$KEYFILE")"
}

case "${1:-all}" in
  init) cmd_init ;;
  build) cmd_build ;;
  flash-usb) cmd_flash_usb ;;
  upload) cmd_upload ;;
  all) cmd_build; cmd_upload ;;
  *) sed -n '2,12p' "$0"; exit 1 ;;
esac
