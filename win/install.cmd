@echo off
rem Install HikyakuRing (replaces the old one). Runs install.ps1 next to this file.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
rem Leave this folder so it can be deleted while the window is still open.
cd /d "%TEMP%"
pause
