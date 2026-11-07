param([string]$Folder = $PSScriptRoot, [string]$ExpectedManifestSha256 = '')
$ErrorActionPreference = 'Stop'
$resourceRoot = [IO.Path]::GetFullPath((Join-Path $Folder 'Resources'))
if ((Get-Item -LiteralPath $resourceRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked resource roots are not accepted.' }
$manifestPath = Join-Path $Folder 'resource-manifest.json'
$manifestHash = (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant()
$appManifestPath = Join-Path $Folder 'build-manifest.json'
if (Test-Path -LiteralPath $appManifestPath) {
    $app = Get-Content -LiteralPath $appManifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($app.resourcePack.manifestSha256 -ne $manifestHash) { throw 'Resource manifest differs from the app build receipt.' }
}
if ($ExpectedManifestSha256 -and $manifestHash -ne $ExpectedManifestSha256.ToLowerInvariant()) { throw 'Resource manifest differs from the expected hash.' }
$pack = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($pack.schema -ne 'quran-mushaf-resource-pack-v1' -or $pack.files.Count -ne 1569 -or $pack.resources.Count -ne 1567) { throw 'Unexpected resource pack schema or file count.' }
$seen = @{}
$totalBytes = 0L
foreach ($entry in $pack.files) {
    if (-not $entry.path -or $entry.path -match '[:\\]' -or ($entry.path.Split('/') | Where-Object { $_ -eq '' -or $_ -eq '.' -or $_ -eq '..' })) { throw 'Unsafe resource path.' }
    if ($seen.ContainsKey($entry.path)) { throw 'Duplicate resource path.' }
    $path = [IO.Path]::GetFullPath((Join-Path $resourceRoot $entry.path))
    if (-not $path.StartsWith($resourceRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Resource path escapes its folder.' }
    $ancestor = $resourceRoot
    foreach ($part in $entry.path.Split('/')) {
        $ancestor = Join-Path $ancestor $part
        if ((Get-Item -LiteralPath $ancestor).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked resource paths are not accepted.' }
    }
    $file = Get-Item -LiteralPath $path
    if ($file.PSIsContainer -or $file.Length -ne $entry.bytes) { throw "Resource size differs: $($entry.path)" }
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $entry.sha256) { throw "Resource hash differs: $($entry.path)" }
    $seen[$entry.path] = $true
    $totalBytes += $file.Length
}
foreach ($file in Get-ChildItem -LiteralPath $resourceRoot -File -Recurse) {
    $relative = $file.FullName.Substring($resourceRoot.Length + 1).Replace('\','/')
    if (-not $seen.ContainsKey($relative)) { throw "Unexpected resource file: $relative" }
}
[ordered]@{ passed = $true; resourcePackId = $pack.id; manifestSha256 = $manifestHash; files = $seen.Count; bytes = $totalBytes; originalResources = 1567; writesPerformed = $false } | ConvertTo-Json
