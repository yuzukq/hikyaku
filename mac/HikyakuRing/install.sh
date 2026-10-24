#!/bin/sh
# HikyakuRing.app を /Applications に入れる（古いものは置き換える。重複しない）。動いているものは終了させてから起動し直す。
#
#   mac/HikyakuRing/install.sh                         build/HikyakuRing.app を入れる（build.sh --install と同じ）
#   mac/HikyakuRing/install.sh ~/Downloads/HikyakuRing-mac.zip   GitHub の Releases（mac-latest）の zip から入れる
#
# zip から入れるときは、ダウンロードで付く隔離の印（com.apple.quarantine）を外す（自分のアプリなので）。
# Releases の版はアドホック署名なので、入れ替えのたびに Bluetooth の許可を聞き直されることがある（手元の build.sh なら起きない）。
set -eu

cd "$(dirname "$0")"
src=${1:-build/HikyakuRing.app}
dest=/Applications/HikyakuRing.app

case "$src" in
*.zip)
    tmp=$(mktemp -d)
    ditto -x -k "$src" "$tmp"
    src="$tmp/HikyakuRing.app"
    ;;
esac
[ -d "$src" ] || { echo "見つからない: $src" >&2; exit 1; }

pkill -x HikyakuRing || true
while pgrep -x HikyakuRing >/dev/null; do sleep 0.1; done
rm -rf "$dest"
ditto "$src" "$dest"
xattr -dr com.apple.quarantine "$dest" 2>/dev/null || true
echo "installed $dest ($(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$dest/Contents/Info.plist"))"
open "$dest"
