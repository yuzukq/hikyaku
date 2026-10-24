#!/bin/sh
# Windows アプリ（C# / WPF）を、Docker の .NET SDK でビルドする（Linux・Mac で。動かすのは Windows）。
#
#   win/build.sh          テスト（RingCore）→ アプリと試作を out/ に出す
#   win/build.sh test     テストだけ
#
# 出力: win/out/HikyakuRing.exe、win/out/RingProbe.exe（.NET のランタイムを同梱した単一の exe。Windows に .NET を入れなくてよい）
#       win/out/hikyaku-windows.zip（上の 2 つと install.cmd / install.ps1。Windows で展開して install.cmd をダブルクリック）
# Windows 上なら Docker なしで: dotnet test RingCore.Tests; dotnet publish HikyakuRing -c Release -r win-x64 -o out
#
# コンテナの中は root で動かす（一般ユーザーだと .NET の初回設定が名前付きミューテックスで失敗する）。最後に持ち主を戻す。
set -eu
cd "$(dirname "$0")"
mkdir -p .nuget out
cmd='dotnet test RingCore.Tests --nologo -v q'
if [ "${1:-}" != "test" ]; then
    rev=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
    publish="-c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -p:EnableCompressionInSingleFile=true -p:DebugType=none -p:EnableWindowsTargeting=true -p:SourceRevisionId=$rev --nologo -v q"
    cmd="$cmd && rm -rf out/* && dotnet publish HikyakuRing $publish -o out && dotnet publish RingProbe $publish -o out"
fi
owner="$(id -u):$(id -g)"
docker run --rm -v "$PWD":/src -w /src \
    -e DOTNET_CLI_TELEMETRY_OPTOUT=1 -e DOTNET_NOLOGO=1 -e NUGET_PACKAGES=/src/.nuget \
    mcr.microsoft.com/dotnet/sdk:8.0 sh -c "status=0; ($cmd) || status=\$?; chown -R $owner /src; exit \$status"

if [ "${1:-}" != "test" ]; then
    cp install.ps1 install.cmd out/
    (cd out && rm -f hikyaku-windows.zip && python3 -m zipfile -c hikyaku-windows.zip HikyakuRing.exe RingProbe.exe install.cmd install.ps1)
    ls -la out
fi
