# HikyakuRing（Windows）を決まった場所に入れる。古いものは置き換える（重複しない）。
#   置き場所: %LOCALAPPDATA%\Programs\HikyakuRing（HikyakuRing.exe・RingProbe.exe）
#   動いている HikyakuRing は終了させてから置き換え、入れた後に起動する。
#   「Windows の起動時に開始」が有効なら、その登録を新しい場所に向け直す。スタートメニューにショートカットを作る。
# 使い方: zip を展開したフォルダで install.cmd をダブルクリック（このスクリプトと同じフォルダの exe を入れる）。
$ErrorActionPreference = 'Stop'
$src = Split-Path -Parent $MyInvocation.MyCommand.Path
$dest = Join-Path $env:LOCALAPPDATA 'Programs\HikyakuRing'
$exe = Join-Path $dest 'HikyakuRing.exe'

foreach ($name in 'HikyakuRing.exe', 'RingProbe.exe') {
    if (-not (Test-Path (Join-Path $src $name))) { throw "$name が見つからない（zip を展開したフォルダで実行する）" }
}

$running = Get-Process -Name HikyakuRing, RingProbe -ErrorAction SilentlyContinue
if ($running) {
    Write-Host '動いている HikyakuRing を終了します'
    $running | Stop-Process -Force
    $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
}

New-Item -ItemType Directory -Force -Path $dest | Out-Null
foreach ($name in 'HikyakuRing.exe', 'RingProbe.exe') {
    Copy-Item (Join-Path $src $name) $dest -Force
    # ダウンロードで付く「インターネットから来た」印を外す（毎回 SmartScreen に聞かれないように）
    Unblock-File (Join-Path $dest $name)
}

$run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
if (Get-ItemProperty -Path $run -Name HikyakuRing -ErrorAction SilentlyContinue) {
    Set-ItemProperty -Path $run -Name HikyakuRing -Value "`"$exe`""
    Write-Host '起動時に開始する登録を、新しい場所に向け直しました'
}

$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'HikyakuRing.lnk'
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($shortcut)
$link.TargetPath = $exe
$link.WorkingDirectory = $dest
$link.Save()

$version = (Get-Item $exe).VersionInfo.ProductVersion
Write-Host "入れました: $exe（版 $version）"
Start-Process $exe
