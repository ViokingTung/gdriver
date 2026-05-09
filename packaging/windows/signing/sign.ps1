# gDriver Windows Code Signing Helper
#
# Signs all binaries in a gDriver build with an EV code signing certificate.
#
# Usage:
#   .\sign.ps1 -TargetDir "target\release\bundle\nsis"
#   .\sign.ps1 -File "path\to\gDriver_0.1.0_x64-setup.exe"
#
# Prerequisites:
#   - Windows SDK (signtool.exe)
#   - EV Code Signing Certificate (USB token or HSM) or PFX file
#   - Signing config: packaging\windows\signing\sign-config.json
#
param(
    [string] $TargetDir,
    [string] $File
)

$ErrorActionPreference = "Stop"

$ScriptDir  = Split-Path -Parent $MyInvocation.MyCommand.Path
$SignConfig = "$ScriptDir\sign-config.json"

if (-not (Test-Path $SignConfig)) {
    Write-Error "Signing configuration not found: $SignConfig"
    Write-Error "Create packaging/windows/signing/sign-config.json with your certificate details."
    exit 1
}

$config = Get-Content $SignConfig -Raw | ConvertFrom-Json

$certThumbprint = $config.certificate_thumbprint
$certFile       = $config.certificate_file
$certPass       = $config.certificate_password
$timestampUrl   = $config.timestamp_url

if (-not $timestampUrl) { $timestampUrl = "http://timestamp.digicert.com" }

# ── Find signtool ───────────────────────────────────────────────────────
$signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
if (-not $signtool) {
    # Try common SDK paths
    $sdkPaths = @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.0.22621.0\x64\signtool.exe",
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.0.22000.0\x64\signtool.exe",
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.0.19041.0\x64\signtool.exe"
    )
    $signtool = $sdkPaths | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $signtool) {
        Write-Error "signtool.exe not found. Install Windows SDK."
        exit 1
    }
}

# ── Build file list ─────────────────────────────────────────────────────
$files = @()

if ($File) {
    $files += (Resolve-Path $File).Path
}

if ($TargetDir) {
    $files += Get-ChildItem "$TargetDir\*.exe" | ForEach-Object { $_.FullName }
    $files += Get-ChildItem "$TargetDir\*.dll" | ForEach-Object { $_.FullName }
}

if ($files.Count -eq 0) {
    Write-Error "No files to sign. Use -TargetDir or -File."
    exit 1
}

# ── Sign each file ──────────────────────────────────────────────────────
foreach ($f in $files) {
    Write-Host "Signing: $f" -ForegroundColor Green

    $args = @(
        "sign",
        "/fd", "SHA256",
        "/tr", $timestampUrl,
        "/td", "SHA256",
        "/d", "gDriver",
        "/du", "https://github.com/gdriver/gdriver"
    )

    if ($certThumbprint) {
        $args += "/sha1"; $args += $certThumbprint
    }
    elseif ($certFile -and $certPass) {
        $args += "/f"; $args += $certFile
        $args += "/p"; $args += $certPass
    }
    else {
        Write-Error "No valid certificate configuration found."
        exit 1
    }

    $args += $f

    & $signtool $args

    if ($LASTEXITCODE -ne 0) {
        Write-Error "Signing failed for: $f (exit code: $LASTEXITCODE)"
        exit 1
    }
}

# ── Verify signatures ───────────────────────────────────────────────────
Write-Host "`nVerifying signatures..." -ForegroundColor Cyan
foreach ($f in $files) {
    & $signtool verify /pa /v $f 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) {
        Write-Host "  OK  $f" -ForegroundColor Green
    } else {
        Write-Host "  FAIL  $f" -ForegroundColor Red
    }
}

Write-Host "`nSigning complete." -ForegroundColor Green
