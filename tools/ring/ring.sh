#!/bin/bash
# ショートカットリングの確認用クライアントを起動する（OTA ツールの Python 環境の bleak を使う）。
#   tools/ring/ring.sh [--addr FF:E4:05:1A:8F:FF]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VENV="$ROOT/tools/ota/.venv"
if [ ! -x "$VENV/bin/python" ]; then
  python3 -m venv "$VENV"
  "$VENV/bin/pip" install -q bleak
fi
exec "$VENV/bin/python" "$ROOT/tools/ring/ring_view.py" "$@"
