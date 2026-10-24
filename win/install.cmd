@echo off
rem Install HikyakuRing (replaces the old one). Runs install.ps1 next to this file.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
pause
