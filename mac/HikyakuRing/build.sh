#!/bin/sh
# HikyakuRing.app を作る（Bluetooth の許可に Info.plist が要るので、swift run ではなく .app にまとめて起動する）。
#
#   mac/HikyakuRing/build.sh            ビルドして build/HikyakuRing.app を作る
#   mac/HikyakuRing/build.sh --open     作って起動する（動いているものは終了させてから。build/ から起動する）
#   mac/HikyakuRing/build.sh --install  作って /Applications に入れ、起動する（install.sh。普段使いはこれ）
#
# 署名: キーチェーンに「Apple Development」の証明書があればそれで、なければアドホック署名。
# アドホックだと、ビルドのたびに Bluetooth の許可を聞き直されることがある。HIKYAKU_SIGN で証明書を指定できる。
# 版: Info.plist の CFBundleVersion に git のコミット（短い形）を入れる（メニューに表示される）。
set -eu

cd "$(dirname "$0")"
swift build -c release --product HikyakuRing

app=build/HikyakuRing.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$(swift build -c release --show-bin-path)/HikyakuRing" "$app/Contents/MacOS/"
cp Info.plist "$app/Contents/"
rev=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $rev" "$app/Contents/Info.plist"

identity=${HIKYAKU_SIGN:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Apple Development: .*\)"/\1/p' | head -1)}
codesign --force --sign "${identity:--}" "$app"
echo "built $app ($rev, signed: ${identity:-ad-hoc})"

case "${1:-}" in
--open)
    # 終わりきる前に open すると、終了中の古いものを指すだけで新しいものが起動しない
    pkill -x HikyakuRing || true
    while pgrep -x HikyakuRing >/dev/null; do sleep 0.1; done
    open "$app"
    ;;
--install)
    ./install.sh "$app"
    ;;
esac
