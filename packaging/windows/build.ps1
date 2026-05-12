# gDriver Windows Packaging Build Script
#
# Usage: .\build.ps1 [-SkipSign] [-BuildMode <nsis|msi|all>]
#
# Prerequisites:
#   - Rust toolchain (rustup) with MSVC target
#   - Node.js + pnpm
#   - Tauri CLI: cargo install tauri-cli
#   - NSIS: set NSIS_DIR env var, or choco install nsis
#   - (optional) WiX Toolset: choco install wixtoolset
#   - (optional) Code signing certificate in Windows Certificate Store
#
param(
    [switch] $SkipSign,
    [ValidateSet("nsis", "msi", "all")]
    [string] $BuildMode = "nsis"
)

$ErrorActionPreference = "Stop"

# ── Paths ────────────────────────────────────────────────────────────────
$ScriptDir    = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectRoot  = Resolve-Path "$ScriptDir\..\.."
$TauriDir     = "$ProjectRoot\apps\gdriver-app\src-tauri"
$DaemonCrate  = "$ProjectRoot\crates\gdriver-daemon"
$ShellCrate   = "$ProjectRoot\extensions\windows-shell"
$TargetDir    = "$ProjectRoot\target\release"
$ShellTarget  = "$ShellCrate\target\release"
$NsisTemplate = "$ScriptDir\nsis\installer.nsi"
$SigningDir   = "$ScriptDir\signing"

# ── Output colors ────────────────────────────────────────────────────────
function Write-Step { Write-Host "`n[gDriver] $args" -ForegroundColor Green }
function Write-Warn  { Write-Host "[gDriver] WARNING: $args" -ForegroundColor Yellow }
function Write-Err   { Write-Host "[gDriver] ERROR: $args" -ForegroundColor Red }

# ── Build daemon ─────────────────────────────────────────────────────────
function Build-Daemon {
    Write-Step "Building gdriver-daemon (release)..."
    Push-Location $ProjectRoot
    cargo build -p gdriver-daemon --release
    if ($LASTEXITCODE -ne 0) {
        Pop-Location
        throw "Daemon build failed with exit code $LASTEXITCODE"
    }
    Pop-Location

    if (-not (Test-Path "$TargetDir\gdriver-daemon.exe")) {
        throw "Daemon binary not found: $TargetDir\gdriver-daemon.exe"
    }
    Write-Step "  -> $TargetDir\gdriver-daemon.exe"
}

# ── Build shell extension ────────────────────────────────────────────────
function Build-ShellExtension {
    Write-Step "Building Windows Shell Extension DLL (release)..."
    cargo build -p gdriver-shell-extension --release
    if ($LASTEXITCODE -ne 0) {
        throw "Shell extension build failed with exit code $LASTEXITCODE"
    }

    # Cargo workspace puts output in workspace root target dir
    if (-not (Test-Path "$TargetDir\gdriver_shell.dll")) {
        throw "Shell extension DLL not found: $TargetDir\gdriver_shell.dll"
    }
    Write-Step "  -> $TargetDir\gdriver_shell.dll"
}

# ── Verify NSIS compiler ────────────────────────────────────────────────
function Assert-RealNsis {
    # If NSIS_DIR is set, Tauri will use it directly (bypasses download).
    # Verify the compiler is real (not a ~2.5KB stub from GitHub).
    $makensisPath = $null

    if ($env:NSIS_DIR -and (Test-Path "$env:NSIS_DIR\makensis.exe")) {
        $makensisPath = "$env:NSIS_DIR\makensis.exe"
        Write-Step "Using NSIS from NSIS_DIR: $makensisPath"
    }
    elseif (Test-Path "$env:LOCALAPPDATA\tauri\NSIS\makensis.exe") {
        $makensisPath = "$env:LOCALAPPDATA\tauri\NSIS\makensis.exe"
        Write-Step "Using NSIS from Tauri cache: $makensisPath"
    }

    if ($makensisPath) {
        $size = (Get-Item $makensisPath).Length
        Write-Step "  makensis.exe size: $size bytes"
        if ($size -lt 100000) {
            Write-Warn "  makensis.exe appears to be a STUB ($size bytes)."
            Write-Warn "  Set NSIS_DIR to a real NSIS installation."
            Write-Warn "  Download from: https://nsis.sourceforge.io/Download"
        }
    } else {
        Write-Warn "NSIS compiler not found. Tauri will attempt to download."
        Write-Warn "Set NSIS_DIR env var to skip download and use a local NSIS."
    }
}

# ── Preprocess NSIS template ─────────────────────────────────────────────
function Preprocess-NsisTemplate {
    Write-Step "Preprocessing NSIS template..."

    $templateContent = Get-Content $NsisTemplate -Raw

    # NSIS requires double-backslash in paths: D:\foo → D:\\foo
    $daemonAbsPath = (Resolve-Path "$TargetDir\gdriver-daemon.exe").Path -replace '\\', '\\\\'
    $shellAbsPath  = (Resolve-Path "$TargetDir\gdriver_shell.dll").Path -replace '\\', '\\\\'

    Write-Step "  Daemon path: $daemonAbsPath"
    Write-Step "  Shell DLL path: $shellAbsPath"

    $templateContent = $templateContent -replace '__DAEMON_BINARY__', $daemonAbsPath
    $templateContent = $templateContent -replace '__SHELL_DLL__', $shellAbsPath

    # Write preprocessed template next to original
    $processedPath = "$ScriptDir\nsis\installer.processed.nsi"
    $templateContent | Set-Content $processedPath -NoNewline

    Write-Step "  -> $processedPath"
    return $processedPath
}

# ── Update tauri.conf.json to use processed template ─────────────────────
function Set-TauriTemplate {
    param([string] $TemplatePath)

    $tauriConfPath = "$TauriDir\tauri.conf.json"
    $text = Get-Content $tauriConfPath -Raw

    # Use absolute path — Tauri resolves relative paths from cwd, not from tauri.conf.json
    $absTemplatePath = (Resolve-Path $TemplatePath).Path -replace '\\', '/'

    $text = $text -replace '"template"\s*:\s*"[^"]*"', "`"template`": `"$absTemplatePath`""

    Set-Content $tauriConfPath $text -NoNewline
    Write-Step "  Updated tauri.conf.json to use template: $absTemplatePath"
}

# ── Build Tauri app ──────────────────────────────────────────────────────
function Invoke-TauriBuild {
    Write-Step "Building Tauri desktop app (release)..."
    Push-Location "$ProjectRoot\apps\gdriver-app"

    # Ensure frontend deps
    if (-not (Test-Path "node_modules")) {
        pnpm install
        if ($LASTEXITCODE -ne 0) { throw "pnpm install failed with exit code $LASTEXITCODE" }
    }

    # Build frontend
    pnpm build
    if ($LASTEXITCODE -ne 0) { throw "pnpm build failed with exit code $LASTEXITCODE" }

    # Tauri build (uses NSIS from NSIS_DIR if set, otherwise downloads)
    Write-Step "  cargo tauri build --bundles $BuildMode"
    cargo tauri build --bundles $BuildMode
    if ($LASTEXITCODE -ne 0) {
        Pop-Location
        throw "Tauri build failed with exit code $LASTEXITCODE"
    }

    Pop-Location

    # Find the generated installer (Tauri outputs to workspace target dir)
    $bundleDir = "$TargetDir\bundle"
    if (-not (Test-Path $bundleDir)) {
        # Fallback: check app-specific target dir
        $bundleDir = "$TauriDir\target\release\bundle"
    }
    if ($BuildMode -eq "msi") {
        $installer = Get-ChildItem "$bundleDir\msi\*.msi" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    } else {
        $installer = Get-ChildItem "$bundleDir\nsis\*.exe" -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    }

    if ($installer) {
        Write-Step "  Installer: $($installer.FullName)"
    }
    return $installer
}

# ── Restore tauri.conf.json ──────────────────────────────────────────────
function Restore-TauriConf {
    $tauriConfPath = "$TauriDir\tauri.conf.json"
    $text = Get-Content $tauriConfPath -Raw
    $text = $text -replace '"template"\s*:\s*"[^"]*"', '"template": "../../../packaging/windows/nsis/installer.nsi"'
    Set-Content $tauriConfPath $text -NoNewline
}

# ── Code signing ─────────────────────────────────────────────────────────
function Invoke-CodeSign {
    param([string] $InstallerPath)

    if ($SkipSign) {
        Write-Warn "Skipping code signing (--skip-sign)"
        return
    }

    # Check for signing configuration
    $signConfig = "$SigningDir\sign-config.json"
    if (-not (Test-Path $signConfig)) {
        Write-Warn "No signing config at $signConfig; skipping code signing."
        Write-Warn "Create packaging/windows/signing/sign-config.json to enable signing."
        return
    }

    Write-Step "Code signing $InstallerPath ..."

    $config = Get-Content $signConfig -Raw | ConvertFrom-Json
    $certThumbprint = $config.certificate_thumbprint
    $timestampUrl  = $config.timestamp_url
    if (-not $timestampUrl) { $timestampUrl = "http://timestamp.digicert.com" }

    if ($certThumbprint) {
        # Sign using certificate from Windows Certificate Store
        & signtool sign /fd SHA256 `
            /sha1 $certThumbprint `
            /tr $timestampUrl `
            /td SHA256 `
            /d "gDriver" `
            /du "https://github.com/gdriver/gdriver" `
            $InstallerPath
    }
    elseif ($config.certificate_file) {
        # Sign using PFX file + password
        $certFile = $config.certificate_file
        $certPass = $config.certificate_password

        if ($certPass) {
            $securePass = ConvertTo-SecureString $certPass -AsPlainText -Force
        }

        & signtool sign /fd SHA256 `
            /f $certFile `
            /p $certPass `
            /tr $timestampUrl `
            /td SHA256 `
            /d "gDriver" `
            /du "https://github.com/gdriver/gdriver" `
            $InstallerPath
    }
    else {
        Write-Warn "No certificate configured in sign-config.json"
        return
    }

    if ($LASTEXITCODE -eq 0) {
        Write-Step "  Code signing complete."

        # Generate SHA-256 checksum
        $checksum = (Get-FileHash $InstallerPath -Algorithm SHA256).Hash
        "$checksum  $($(Split-Path $InstallerPath -Leaf))" | Out-File "$InstallerPath.sha256" -Encoding ASCII
        Write-Step "  SHA-256: $checksum"
    } else {
        Write-Warn "  Code signing failed (exit code: $LASTEXITCODE)"
    }
}

# ── Main ─────────────────────────────────────────────────────────────────
function Main {
    Write-Step "=== gDriver Windows Packaging ==="
    Write-Step "Project root : $ProjectRoot"
    Write-Step "Build mode   : $BuildMode"

    # 1. Verify NSIS compiler
    if ($BuildMode -ne "msi") {
        Assert-RealNsis
    }

    # 2. Build dependencies
    Build-Daemon
    Build-ShellExtension

    # 3. Preprocess NSIS template
    if ($BuildMode -ne "msi") {
        $processedTemplate = Preprocess-NsisTemplate
        Set-TauriTemplate -TemplatePath $processedTemplate

        # Debug: verify template preprocessing
        if (Test-Path $processedTemplate) {
            Write-Step "  Processed template exists: $processedTemplate"
            Write-Step "  Template size: $((Get-Item $processedTemplate).Length) bytes"
        } else {
            throw "Processed template not found: $processedTemplate"
        }

        # Debug: verify tauri.conf.json was updated
        $tauriConf = Get-Content "$TauriDir\tauri.conf.json" -Raw
        if ($tauriConf -match '"template"\s*:\s*"([^"]+)"') {
            Write-Step "  tauri.conf.json template: $($Matches[1])"
        } else {
            Write-Warn "  Could not find template path in tauri.conf.json"
        }
    }

    # 4. Build Tauri app + installer
    try {
        $installer = Invoke-TauriBuild

        # 5. Code sign
        if ($installer) {
            Invoke-CodeSign -InstallerPath $installer.FullName
        }
    }
    finally {
        # Always restore original config
        if ($BuildMode -ne "msi") {
            Restore-TauriConf
        }
    }

    Write-Step "=== Packaging complete ==="
    Write-Step "Output: $TauriDir\target\release\bundle\"
}

Main
