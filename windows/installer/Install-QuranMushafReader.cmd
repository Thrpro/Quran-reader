@echo off
setlocal

set "SOURCE=%~dp0"
set "TARGET=%LOCALAPPDATA%\Programs\QuranMushafReader"

"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -Command ^
  "$source = $env:SOURCE; " ^
  "$target = $env:TARGET; " ^
  "$appNames = @('Quran Mushaf Reader','QuranMushafReader','Quran App Mushaf','Quran Mushaf'); " ^
  "Get-Process QuranMushaf.Windows -ErrorAction SilentlyContinue | Stop-Process -Force; " ^
  "Start-Sleep -Milliseconds 500; " ^
  "$oldInstallPaths = @($target, (Join-Path $env:LOCALAPPDATA 'Programs\Quran Mushaf Reader'), (Join-Path $env:LOCALAPPDATA 'QuranMushafReader'), (Join-Path $env:LOCALAPPDATA 'Quran Mushaf Reader')); " ^
  "$sourceFull = [System.IO.Path]::GetFullPath($source).TrimEnd('\'); " ^
  "$oldInstallPaths | Select-Object -Unique | ForEach-Object { " ^
  "  $path = [System.IO.Path]::GetFullPath($_).TrimEnd('\'); " ^
  "  if ((Test-Path $path) -and ($path -ne $sourceFull)) { Remove-Item -LiteralPath $path -Recurse -Force; } " ^
  "}; " ^
  "New-Item -ItemType Directory -Force -Path $target | Out-Null; " ^
  "if ([System.IO.Path]::GetFullPath($target).TrimEnd('\') -ne $sourceFull) { Copy-Item -Path (Join-Path $source '*') -Destination $target -Recurse -Force; } " ^
  "$shell = New-Object -ComObject WScript.Shell; " ^
  "$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'; " ^
  "$desktop = [Environment]::GetFolderPath('DesktopDirectory'); " ^
  "New-Item -ItemType Directory -Force -Path $startMenu | Out-Null; " ^
  "$shortcutRoots = @($desktop, (Join-Path $env:USERPROFILE 'Desktop'), $startMenu); " ^
  "$shortcutRoots | Where-Object { Test-Path $_ } | ForEach-Object { " ^
  "  Get-ChildItem -LiteralPath $_ -Filter '*.lnk' -Recurse -ErrorAction SilentlyContinue | Where-Object { " ^
  "    $name = $_.BaseName; " ^
  "    ($appNames | Where-Object { $name -like ('*' + $_ + '*') }) -or $name -like '*QuranMushaf*' " ^
  "  } | Remove-Item -Force -ErrorAction SilentlyContinue; " ^
  "}; " ^
  "$shortcutPaths = @(); " ^
  "$shortcutPaths += Join-Path $desktop 'Quran Mushaf Reader.lnk'; " ^
  "$shortcutPaths += Join-Path $startMenu 'Quran Mushaf Reader.lnk'; " ^
  "$shortcutPaths | Select-Object -Unique | ForEach-Object { " ^
  "  $shortcutDirectory = Split-Path $_; " ^
  "  if ($shortcutDirectory) { New-Item -ItemType Directory -Force -Path $shortcutDirectory | Out-Null; } " ^
  "  $shortcut = $shell.CreateShortcut($_); " ^
  "  $shortcut.TargetPath = Join-Path $target 'QuranMushaf.Windows.exe'; " ^
  "  $shortcut.WorkingDirectory = $target; " ^
  "  $shortcut.IconLocation = Join-Path $target 'Assets\app.ico'; " ^
  "  $shortcut.Save(); " ^
  "}; " ^
  "Start-Process (Join-Path $target 'QuranMushaf.Windows.exe')"

if errorlevel 1 (
  echo Installation failed.
  pause
  exit /b 1
)

echo Quran Mushaf Reader installed.
timeout /t 2 >nul
