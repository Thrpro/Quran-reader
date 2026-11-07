<#
Assembles a release of the native reader from a built executable:
the app folder, the ZIP, the installer and SHA256SUMS.txt.

The Quran resources are not built here. They come from an earlier release ZIP,
pinned by its SHA-256, and are checked again against the resource manifest,
whose own SHA-256 is pinned too. Nothing in Resources is modified.

Used by .github/workflows/native-reader.yml and runnable locally:
  ./package.ps1 -Exe <built exe> -Version 0.17.0 -OutDir <folder>
#>
param(
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $Version,
    [Parameter(Mandatory)] [string] $OutDir,
    # Release ZIP that carries the verified resource pack, and its SHA-256.
    [string] $ResourceZip = 'https://github.com/Thrpro/Quran-reader/releases/download/v0.16.0/QuranMushafReader-Windows-x64-0.16.0.zip',
    [string] $ResourceZipSha256 = '97a8d7e7be094996e68ab369bff6303c8588463cf247a9a3a4c900c007fc58e0',
    # Identity of the resource pack qul-aeea44b86fad2600.
    [string] $ManifestSha256 = '378e25fde0bbbb24e185393cbc16a704c56dd10b47391cfaae2347e9e90ccc8a',
    [string] $RedistDir = '',
    [string] $Iscc = '',
    # Stop after the app folder, for example to sign the executable first.
    [switch] $AppOnly,
    # Reuse an app folder made by an earlier -AppOnly run (with a signed executable).
    [switch] $FromAppFolder
)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must look like 1.2.3, not '$Version'." }
$OutDir = [IO.Path]::GetFullPath($OutDir)
New-Item -ItemType Directory -Force $OutDir | Out-Null
$appName = "Quran-Mushaf-Reader-$Version"
$app = Join-Path $OutDir $appName

function Get-Sha256([string] $Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }

if (-not $FromAppFolder) {
    if (Test-Path $app) { Remove-Item $app -Recurse -Force }
    New-Item -ItemType Directory $app | Out-Null

    # 1. The pinned resource base.
    $work = Join-Path ([IO.Path]::GetTempPath()) ("qr-package-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory $work | Out-Null
    try {
        $zip = Join-Path $work 'base.zip'
        if ($ResourceZip -match '^https://') {
            Invoke-WebRequest -Uri $ResourceZip -OutFile $zip -UseBasicParsing
        } else {
            Copy-Item -LiteralPath $ResourceZip -Destination $zip
        }
        $actual = Get-Sha256 $zip
        if ($actual -ne $ResourceZipSha256) { throw "Resource base ZIP SHA-256 is $actual, expected $ResourceZipSha256." }
        Expand-Archive -LiteralPath $zip -DestinationPath (Join-Path $work 'x')
        $base = Get-ChildItem (Join-Path $work 'x') -Directory | Select-Object -First 1
        foreach ($item in 'Resources', 'licenses', 'resource-manifest.json', 'Verify-Quran-Resources.ps1') {
            Copy-Item -LiteralPath (Join-Path $base.FullName $item) -Destination $app -Recurse
        }
    } finally {
        Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    }

    # 2. Check the Quran files against the pinned manifest, without changing them.
    $manifest = Get-Sha256 (Join-Path $app 'resource-manifest.json')
    if ($manifest -ne $ManifestSha256) { throw "Resource manifest SHA-256 is $manifest, expected $ManifestSha256." }
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $app 'Verify-Quran-Resources.ps1') -Folder $app -ExpectedManifestSha256 $ManifestSha256 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Quran resource verification failed.' }

    # 3. The app, its runtime and its notes.
    Copy-Item -LiteralPath $Exe -Destination (Join-Path $app 'QuranMushaf.SlintReader.exe')
    if (-not $RedistDir) {
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
        $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        $RedistDir = Get-ChildItem (Join-Path $vs 'VC\Redist\MSVC') -Directory |
            Where-Object Name -match '^\d+\.\d+\.\d+$' |
            Sort-Object { [version]$_.Name } -Descending |
            ForEach-Object { Get-ChildItem (Join-Path $_.FullName 'x64') -Directory -Filter 'Microsoft.VC*.CRT' -ErrorAction SilentlyContinue } |
            Select-Object -First 1 -ExpandProperty FullName
    }
    foreach ($dll in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll') {
        Copy-Item -LiteralPath (Join-Path $RedistDir $dll) -Destination $app
    }
    Copy-Item -LiteralPath (Join-Path $here 'Microsoft-Visual-C++-Runtime.txt') -Destination (Join-Path $app 'licenses')
    (Get-Content -LiteralPath (Join-Path $here 'START-HERE.md') -Raw).Replace('{VERSION}', $Version) |
        Set-Content -LiteralPath (Join-Path $app 'START-HERE.md') -Encoding utf8 -NoNewline
    Write-Host "App folder ready: $app"
    if ($AppOnly) { return }
}

# 4. ZIP, installer and checksums.
$release = Join-Path $OutDir 'release'
if (Test-Path $release) { Remove-Item $release -Recurse -Force }
New-Item -ItemType Directory $release | Out-Null
$zipName = "QuranMushafReader-Windows-x64-$Version.zip"
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$archive = [IO.Compression.ZipFile]::Open((Join-Path $release $zipName), 'Create')
try {
    Get-ChildItem $app -Recurse -File | Where-Object { $_.FullName -notmatch '\\SlintProfile\\' } | ForEach-Object {
        $entry = "$appName/" + $_.FullName.Substring($app.Length + 1).Replace('\', '/')
        [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $_.FullName, $entry, 'Optimal') | Out-Null
    }
} finally { $archive.Dispose() }

if (-not $Iscc) {
    $Iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $Iscc) { throw 'Inno Setup 6 (ISCC.exe) was not found.' }
& $Iscc /Q "/DAppVersion=$Version" "/DAppDir=$app" "/DOutputDir=$release" (Join-Path $here 'QuranMushafReader.iss')
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed with exit code $LASTEXITCODE." }

$sums = Get-ChildItem $release -File | Where-Object Extension -in '.zip', '.exe' | Sort-Object Name |
    ForEach-Object { "$(Get-Sha256 $_.FullName)  $($_.Name)" }
[IO.File]::WriteAllText((Join-Path $release 'SHA256SUMS.txt'), ($sums -join "`n") + "`n")
Get-ChildItem $release | ForEach-Object { '{0}  {1:N1} MB' -f $_.Name, ($_.Length / 1MB) }
