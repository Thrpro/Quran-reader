@echo off
setlocal

set "TARGET=%LOCALAPPDATA%\Programs\QuranMushafReader"

"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -Command ^
  "$target = $env:TARGET; " ^
  "$appNames = @('Quran Mushaf Reader','QuranMushafReader','Quran App Mushaf','Quran Mushaf'); " ^
  "Get-Process QuranMushaf.Windows -ErrorAction SilentlyContinue | Stop-Process -Force; " ^
  "$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'; " ^
  "$shortcutRoots = @(([Environment]::GetFolderPath('DesktopDirectory')), (Join-Path $env:USERPROFILE 'Desktop'), $startMenu); " ^
  "$shortcutRoots | Where-Object { Test-Path $_ } | ForEach-Object { " ^
  "  Get-ChildItem -LiteralPath $_ -Filter '*.lnk' -Recurse -ErrorAction SilentlyContinue | Where-Object { " ^
  "    $name = $_.BaseName; " ^
  "    ($appNames | Where-Object { $name -like ('*' + $_ + '*') }) -or $name -like '*QuranMushaf*' " ^
  "  } | Remove-Item -Force -ErrorAction SilentlyContinue; " ^
  "}; " ^
  "$oldInstallPaths = @($target, (Join-Path $env:LOCALAPPDATA 'Programs\Quran Mushaf Reader'), (Join-Path $env:LOCALAPPDATA 'QuranMushafReader'), (Join-Path $env:LOCALAPPDATA 'Quran Mushaf Reader')); " ^
  "$oldInstallPaths | Select-Object -Unique | ForEach-Object { if (Test-Path $_) { Remove-Item -LiteralPath $_ -Recurse -Force; } }"

if errorlevel 1 (
  echo Uninstall failed.
  pause
  exit /b 1
)

echo Quran Mushaf Reader removed.
timeout /t 2 >nul
