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
    # Tauri caches NSIS at %LOCALAPPDATA%\tauri\NSIS.
    # GitHub binary-releases and SourceForge both ship a STUB makensis.exe (~2.5KB)
    # that cannot compile. The real console compiler is NSIS.exe.
    $makensisPath = "$env:LOCALAPPDATA\tauri\NSIS\makensis.exe"

    if (Test-Path $makensisPath) {
        $size = (Get-Item $makensisPath).Length
        Write-Step "NSIS compiler: $makensisPath ($size bytes)"
        if ($size -lt 50000) {
            Write-Warn "  makensis.exe is a STUB ($size bytes). Replacing with NSIS.exe..."
            $nsisExe = "$env:LOCALAPPDATA\tauri\NSIS\NSIS.exe"
            if (Test-Path $nsisExe) {
                $nsisSize = (Get-Item $nsisExe).Length
                Copy-Item $nsisExe $makensisPath -Force
                Copy-Item $nsisExe "$env:LOCALAPPDATA\tauri\NSIS\Bin\makensis.exe" -Force
                Write-Step "  Replaced stub with NSIS.exe ($nsisSize bytes)"
            } else {
                Write-Err "  NSIS.exe not found! Cannot fix stub compiler."
            }
        }
    } else {
        Write-Warn "NSIS compiler not found at: $makensisPath"
        Write-Warn "Tauri will download from GitHub (contains stub compiler)."
    }
}

# ── Preprocess NSIS template ─────────────────────────────────────────────
function Preprocess-NsisTemplate {
    Write-Step "Preprocessing NSIS template..."

    $templateContent = Get-Content $NsisTemplate -Raw

    # NSIS needs \\ for a literal backslash. Use .Replace() for literal replacement.
    $daemonAbsPath = (Resolve-Path "$TargetDir\gdriver-daemon.exe").Path.Replace('\', '\\')
    $shellAbsPath  = (Resolve-Path "$TargetDir\gdriver_shell.dll").Path.Replace('\', '\\')

    Write-Step "  Daemon path: $daemonAbsPath"
    Write-Step "  Shell DLL path: $shellAbsPath"

    # Verify files exist before preprocessing
    $daemonFile = "$TargetDir\gdriver-daemon.exe"
    $shellFile = "$TargetDir\gdriver_shell.dll"
    if (-not (Test-Path $daemonFile)) { throw "Daemon binary not found: $daemonFile" }
    if (-not (Test-Path $shellFile)) { throw "Shell DLL not found: $shellFile" }
    Write-Step "  Daemon exists: $((Get-Item $daemonFile).Length) bytes"
    Write-Step "  Shell DLL exists: $((Get-Item $shellFile).Length) bytes"

    $templateContent = $templateContent -replace '__DAEMON_BINARY__', $daemonAbsPath
    $templateContent = $templateContent -replace '__SHELL_DLL__', $shellAbsPath

    # Write preprocessed template next to original
    $processedPath = "$ScriptDir\nsis\installer.processed.nsi"
    $templateContent | Set-Content $processedPath -NoNewline

    Write-Step "  -> $processedPath"
    return $processedPath
}

# ── Update tauri.conf.json for NSIS build ─────────────────────────────────
function Set-TauriNsisConfig {
    param([string] $TemplatePath)

    $tauriConfPath = "$TauriDir\tauri.conf.json"
    $text = Get-Content $tauriConfPath -Raw

    # Use absolute path — Tauri resolves relative paths from cwd, not from tauri.conf.json
    $absTemplatePath = (Resolve-Path $TemplatePath).Path -replace '\\', '/'

    $text = $text -replace '"template"\s*:\s*"[^"]*"', "`"template`": `"$absTemplatePath`""

    # Fix installerIcon path — Tauri's NSIS bundler uses dunce::canonicalize()
    # which resolves relative to CWD (apps/gdriver-app), not tauri.conf.json dir.
    # Make it absolute so it works regardless of CWD.
    $iconPath = "$TauriDir\icons\icon.ico"
    if (Test-Path $iconPath) {
        $absIconPath = (Resolve-Path $iconPath).Path -replace '\\', '/'
        $text = $text -replace '"installerIcon"\s*:\s*"[^"]*"', "`"installerIcon`": `"$absIconPath`""
        Write-Step "  installerIcon: $absIconPath"
    }

    Set-Content $tauriConfPath $text -NoNewline
    Write-Step "  Updated tauri.conf.json for NSIS build"
    Write-Step "  template: $absTemplatePath"
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
    # Verify makensis.exe still exists right before cargo tauri build
    $makensisCheck = "$env:LOCALAPPDATA\tauri\NSIS\makensis.exe"
    if (Test-Path $makensisCheck) {
        Write-Step "  makensis.exe before build: $((Get-Item $makensisCheck).Length) bytes"
    } else {
        Write-Err "  makensis.exe MISSING before cargo tauri build!"
    }
    # Also check if daemon binary still exists
    $daemonCheck = "$ProjectRoot\target\release\gdriver-daemon.exe"
    if (Test-Path $daemonCheck) {
        Write-Step "  daemon before build: $((Get-Item $daemonCheck).Length) bytes"
    } else {
        Write-Err "  daemon MISSING before cargo tauri build!"
    }
    Write-Step "  cargo tauri build --bundles $BuildMode"

    # Check NSIS output directory
    $nsisOutputDir = "$ProjectRoot\target\release\nsis\x64"
    Write-Step "  NSIS output dir: $nsisOutputDir (exists: $(Test-Path $nsisOutputDir))"

    cargo tauri build --bundles $BuildMode 2>&1 | ForEach-Object {
        $line = $_.ToString()
        if ($line -match "nsis|makensis|NSIS|bundle|Error|error|failed|not found|File:") {
            Write-Step "  TAURI: $line"
        }
    }

    # If build failed, dump the rendered NSIS script for debugging
    if ($LASTEXITCODE -ne 0) {
        $renderedScript = "$nsisOutputDir\installer.nsi"
        if (Test-Path $renderedScript) {
            Write-Step "  Rendered NSIS script exists. First 30 lines:"
            Get-Content $renderedScript -TotalCount 30 | ForEach-Object { Write-Step "    $_" }
        } else {
            Write-Step "  Rendered NSIS script NOT found at: $renderedScript"
        }
    }
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
    $text = $text -replace '"installerIcon"\s*:\s*"[^"]*"', '"installerIcon": "icons/icon.ico"'
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
        Set-TauriNsisConfig -TemplatePath $processedTemplate

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
            # Verify template file exists
            $tplPath = $Matches[1]
            if (Test-Path $tplPath) {
                Write-Step "  Template file exists: $tplPath"
                # Show first few lines with daemon/shell paths
                Get-Content $tplPath | Select-String "DAEMON_BINARY|SHELL_DLL" | ForEach-Object {
                    Write-Step "    $_"
                }
            } else {
                Write-Err "  Template file NOT found: $tplPath"
            }
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
