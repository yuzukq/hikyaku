#!/bin/sh
# HikyakuRing.app を作る（Bluetooth の許可に Info.plist が要るので、swift run ではなく .app にまとめて起動する）。
#
#   mac/HikyakuRing/build.sh          ビルドして build/HikyakuRing.app を作る
#   mac/HikyakuRing/build.sh --open   作って起動する（動いているものは終了させてから）
#
# 署名: キーチェーンに「Apple Development」の証明書があればそれで、なければアドホック署名。
# アドホックだと、ビルドのたびに Bluetooth の許可を聞き直されることがある。HIKYAKU_SIGN で証明書を指定できる。
set -eu

cd "$(dirname "$0")"
swift build -c release --product HikyakuRing

app=build/HikyakuRing.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$(swift build -c release --show-bin-path)/HikyakuRing" "$app/Contents/MacOS/"
cp Info.plist "$app/Contents/"

identity=${HIKYAKU_SIGN:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Apple Development: .*\)"/\1/p' | head -1)}
codesign --force --sign "${identity:--}" "$app"
echo "built $app (signed: ${identity:-ad-hoc})"

if [ "${1:-}" = "--open" ]; then
    # 終わりきる前に open すると、終了中の古いものを指すだけで新しいものが起動しない
    pkill -x HikyakuRing || true
    while pgrep -x HikyakuRing >/dev/null; do sleep 0.1; done
    open "$app"
fi
