param([Parameter(Mandatory=$true)][string]$Path)
$ErrorActionPreference = 'Stop'
$certificateThumbprint = $env:QURAN_SIGNING_CERTIFICATE
if (-not $certificateThumbprint) {
    if ($env:QURAN_REQUIRE_SIGNING -eq '1') { throw 'Public releases require QURAN_SIGNING_CERTIFICATE (trusted CurrentUser code-signing certificate thumbprint).' }
    Write-Host 'Local build is unsigned. Set QURAN_SIGNING_CERTIFICATE and QURAN_REQUIRE_SIGNING=1 for a public release.'
    exit 0
}
if ($certificateThumbprint -notmatch '^[A-Fa-f0-9]{40}$') { throw 'Invalid certificate thumbprint.' }
$certificate = Get-Item -LiteralPath "Cert:\CurrentUser\My\$certificateThumbprint"
if (-not $certificate.HasPrivateKey -or $certificate.NotAfter -le (Get-Date)) { throw 'Signing certificate needs a private key and must not be expired.' }
if (-not ($certificate.EnhancedKeyUsageList | Where-Object { $_.ObjectId -eq '1.3.6.1.5.5.7.3.3' })) { throw 'Certificate must support code signing.' }
$signToolPath = $env:QURAN_SIGNTOOL
if (-not $signToolPath) {
    $sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $signToolPath = Get-ChildItem -LiteralPath $sdkRoot -Directory | Sort-Object Name -Descending |
        ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}
if (-not $signToolPath -or -not (Test-Path -LiteralPath $signToolPath)) { throw 'Install Windows SDK signing tools or set QURAN_SIGNTOOL.' }
& $signToolPath sign /fd SHA256 /sha1 $certificateThumbprint /tr 'http://timestamp.digicert.com' /td SHA256 $Path
if ($LASTEXITCODE -ne 0) { throw 'Authenticode signing or timestamping failed.' }
& $signToolPath verify /pa /all $Path
if ($LASTEXITCODE -ne 0) { throw 'Publisher signature verification failed.' }
$signature = Get-AuthenticodeSignature -LiteralPath $Path
if ($signature.Status -ne 'Valid' -or -not $signature.TimeStamperCertificate) { throw 'A valid, timestamped publisher signature is required.' }
