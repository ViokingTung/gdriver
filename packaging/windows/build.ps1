# gDriver Windows Packaging Build Script
#
# Usage: .\build.ps1 [-SkipSign] [-BuildMode <nsis|msi|all>]
#
# Prerequisites:
#   - Rust toolchain (rustup) with MSVC target
#   - Node.js + pnpm
#   - Tauri CLI: cargo install tauri-cli
#   - NSIS: choco install nsis  (or from https://nsis.sourceforge.io/)
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

# ── Preprocess NSIS template (fully manual, bypasses Tauri NSIS bundler) ──
function Preprocess-NsisTemplate {
    param([string] $BundleDir)

    Write-Step "Preprocessing NSIS template (full manual mode)..."

    $templateContent = Get-Content $NsisTemplate -Raw

    # NSIS requires double-backslash in paths: D:\foo → D:\\foo
    $daemonAbsPath = (Resolve-Path "$TargetDir\gdriver-daemon.exe").Path -replace '\\', '\\\\'
    $shellAbsPath  = (Resolve-Path "$TargetDir\gdriver_shell.dll").Path -replace '\\', '\\\\'

    # Read product info from tauri.conf.json
    $tauriConf = Get-Content "$TauriDir\tauri.conf.json" -Raw | ConvertFrom-Json
    $productName = $tauriConf.productName
    $version     = $tauriConf.version

    # Main binary name (Cargo package name, lowercase with hyphens → underscores)
    $mainBinaryName = "gdriver"

    # Resolve installer icon path
    $iconRelPath = $tauriConf.bundle.windows.nsis.installerIcon
    $iconAbsPath = (Resolve-Path "$TauriDir\$iconRelPath").Path -replace '\\', '\\\\'

    Write-Step "  Product: $productName $version"
    Write-Step "  Daemon path: $daemonAbsPath"
    Write-Step "  Shell DLL path: $shellAbsPath"
    Write-Step "  Icon path: $iconAbsPath"

    # ── Replace custom placeholders ──
    $templateContent = $templateContent -replace '__DAEMON_BINARY__', $daemonAbsPath
    $templateContent = $templateContent -replace '__SHELL_DLL__', $shellAbsPath

    # ── Replace Tauri built-in placeholders ──
    $templateContent = $templateContent -replace '\{\{product_name\}\}', $productName
    $templateContent = $templateContent -replace '\{\{version\}\}', $version
    $templateContent = $templateContent -replace '\{\{main_binary_name\}\}', $mainBinaryName
    $templateContent = $templateContent -replace '\{\{installer_icon\}\}', $iconAbsPath

    # ── Generate {{#each binaries}} section ──
    # Binaries = main executable only (daemon and shell DLL are handled separately)
    $binaryFile = "$TargetDir\gdriver.exe"
    if (-not (Test-Path $binaryFile)) {
        throw "Main binary not found: $binaryFile"
    }
    $binaryAbsPath = $binaryFile -replace '\\', '\\\\'
    $binariesFile = "    File `"/oname=$mainBinaryName.exe`" `"$binaryAbsPath`""
    $binariesDelete = "    Delete `"`$INSTDIR\\$mainBinaryName.exe`""

    # Replace first occurrence (install section) with File command
    $eachBinariesPattern = '(?s)\{\{#each binaries\}\}.*?\{\{/each\}\}'
    $templateContent = [regex]::Replace($templateContent, $eachBinariesPattern, $binariesFile + "`n", 1)
    # Replace second occurrence (uninstall section) with Delete command
    $templateContent = [regex]::Replace($templateContent, $eachBinariesPattern, $binariesDelete + "`n", 1)

    # ── Generate {{#each resources}} section ──
    # Resources = all files in bundle output EXCEPT the main .exe
    $resourceFiles = @()
    if (Test-Path $BundleDir) {
        $resourceFiles = Get-ChildItem $BundleDir -Recurse -File |
            Where-Object { $_.Extension -ne ".exe" -and $_.Extension -ne ".nsi" } |
            ForEach-Object { $_.FullName }
    }

    if ($resourceFiles.Count -gt 0) {
        $resourcesFileLines = @()
        $resourcesDeleteLines = @()
        foreach ($rf in $resourceFiles) {
            $absPath = $rf -replace '\\', '\\\\'
            $fileName = Split-Path $rf -Leaf
            $resourcesFileLines += "    File `"/oname=$fileName`" `"$absPath`""
            $resourcesDeleteLines += "    Delete `"`$INSTDIR\\$fileName`""
        }
        $resourcesFileSection = $resourcesFileLines -join "`n"
        $resourcesDeleteSection = $resourcesDeleteLines -join "`n"
    } else {
        $resourcesFileSection = "    ; No additional resources"
        $resourcesDeleteSection = "    ; No additional resources"
    }

    # Replace first occurrence (install section) with File commands
    $eachResourcesPattern = '(?s)\{\{#each resources\}\}.*?\{\{/each\}\}'
    $templateContent = [regex]::Replace($templateContent, $eachResourcesPattern, $resourcesFileSection + "`n", 1)
    # Replace second occurrence (uninstall section) with Delete commands
    $templateContent = [regex]::Replace($templateContent, $eachResourcesPattern, $resourcesDeleteSection + "`n", 1)

    # Write preprocessed template
    $processedPath = "$ScriptDir\nsis\installer.processed.nsi"
    $templateContent | Set-Content $processedPath -NoNewline

    Write-Step "  -> $processedPath"
    Write-Step "  Template size: $((Get-Item $processedPath).Length) bytes"
    return $processedPath
}

# ── Build Tauri app (without NSIS bundling) ───────────────────────────────
function Invoke-TauriBuild {
    Write-Step "Building Tauri desktop app (release)..."

    # For NSIS mode, build only the app bundle (not NSIS).
    # NSIS compilation is done separately with the real compiler.
    $tauriBundleMode = if ($BuildMode -eq "nsis") { "app" } else { $BuildMode }

    Push-Location "$ProjectRoot\apps\gdriver-app"

    # Ensure frontend deps
    if (-not (Test-Path "node_modules")) {
        pnpm install
        if ($LASTEXITCODE -ne 0) { throw "pnpm install failed with exit code $LASTEXITCODE" }
    }

    # Build frontend
    pnpm build
    if ($LASTEXITCODE -ne 0) { throw "pnpm build failed with exit code $LASTEXITCODE" }

    # Tauri build
    Write-Step "  cargo tauri build --bundles $tauriBundleMode"
    cargo tauri build --bundles $tauriBundleMode
    if ($LASTEXITCODE -ne 0) {
        Pop-Location
        throw "Tauri build failed with exit code $LASTEXITCODE"
    }

    Pop-Location

    # Find the bundle output directory
    $bundleDir = "$TargetDir\bundle"
    if (-not (Test-Path $bundleDir)) {
        $bundleDir = "$TauriDir\target\release\bundle"
    }

    if ($BuildMode -eq "msi") {
        $installer = Get-ChildItem "$bundleDir\msi\*.msi" -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
    } elseif ($BuildMode -eq "nsis") {
        # For NSIS mode, we compile separately — return the app bundle dir
        $installer = $null
    } else {
        $installer = Get-ChildItem "$bundleDir\nsis\*.exe" -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
    }

    if ($installer) {
        Write-Step "  Installer: $($installer.FullName)"
    }
    return $installer
}

# ── Compile NSIS installer with real compiler ─────────────────────────────
function Invoke-NsisCompile {
    param([string] $ProcessedTemplate)

    Write-Step "Compiling NSIS installer with real compiler..."

    # Find real NSIS compiler
    # Priority: 1) NSIS_DIR env var, 2) Tauri cache (after CI replacement), 3) PATH
    $makensisPath = $null

    if ($env:NSIS_DIR -and (Test-Path "$env:NSIS_DIR\makensis.exe")) {
        $makensisPath = "$env:NSIS_DIR\makensis.exe"
        Write-Step "  Using NSIS from NSIS_DIR: $makensisPath"
    }
    elseif (Test-Path "$env:LOCALAPPDATA\tauri\NSIS\makensis.exe") {
        $makensisPath = "$env:LOCALAPPDATA\tauri\NSIS\makensis.exe"
        Write-Step "  Using NSIS from Tauri cache: $makensisPath"
    }
    else {
        $makensisPath = (Get-Command makensis.exe -ErrorAction SilentlyContinue).Source
        if ($makensisPath) {
            Write-Step "  Using NSIS from PATH: $makensisPath"
        }
    }

    if (-not $makensisPath) {
        throw "NSIS compiler (makensis.exe) not found. Install NSIS or set NSIS_DIR."
    }

    # Verify it's the real compiler (stub is ~2.5KB, real is >1MB)
    $compilerSize = (Get-Item $makensisPath).Length
    Write-Step "  Compiler size: $compilerSize bytes"
    if ($compilerSize -lt 100000) {
        throw "makensis.exe appears to be a stub ($compilerSize bytes). Need real NSIS compiler."
    }

    # Run makensis
    Write-Step "  Running: & `"$makensisPath`" /V3 /INPUTCHARSET UTF8 /OUTPUTCHARSET UTF8 `"$ProcessedTemplate`""
    & $makensisPath /V3 /INPUTCHARSET UTF8 /OUTPUTCHARSET UTF8 $ProcessedTemplate
    if ($LASTEXITCODE -ne 0) {
        throw "NSIS compilation failed with exit code $LASTEXITCODE"
    }

    # Find the generated installer
    # NSIS outputs to the directory of the .nsi script (packaging/windows/nsis/)
    $installer = Get-ChildItem "$ScriptDir\nsis\*.exe" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1

    if (-not $installer) {
        # Also check the current directory (where makensis ran)
        $installer = Get-ChildItem "*.exe" -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -match "setup|install" } |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
    }

    if ($installer) {
        Write-Step "  Installer: $($installer.FullName)"
    } else {
        Write-Warn "  Installer .exe not found after compilation"
    }

    return $installer
}

# ── Restore tauri.conf.json ──────────────────────────────────────────────
function Restore-TauriConf {
    # No longer needed — we don't modify tauri.conf.json
    # (kept as no-op for backward compatibility)
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

    # 1. Build dependencies
    Build-Daemon
    Build-ShellExtension

    # 2. Build Tauri app
    # For NSIS mode: build only the app bundle (--bundles app), then compile
    # NSIS separately with the real compiler (bypasses Tauri's stub NSIS).
    $installer = Invoke-TauriBuild

    # 3. For NSIS mode: preprocess template and compile with real NSIS
    if ($BuildMode -eq "nsis") {
        # Find the Tauri bundle output directory (contains gdriver.exe + resources)
        $bundleDir = "$TargetDir\bundle\app"
        if (-not (Test-Path $bundleDir)) {
            $bundleDir = "$TauriDir\target\release\bundle\app"
        }
        if (-not (Test-Path $bundleDir)) {
            # Fallback: check for any bundle output
            $bundleDir = "$TargetDir\bundle"
            if (-not (Test-Path $bundleDir)) {
                $bundleDir = "$TauriDir\target\release\bundle"
            }
        }

        Write-Step "Bundle directory: $bundleDir"
        if (Test-Path $bundleDir) {
            Get-ChildItem $bundleDir -Recurse -File |
                Select-Object FullName, Length |
                Format-Table -AutoSize
        }

        # Preprocess NSIS template (replaces ALL Tauri + custom variables)
        $processedTemplate = Preprocess-NsisTemplate -BundleDir $bundleDir

        # Compile NSIS installer with real compiler
        $installer = Invoke-NsisCompile -ProcessedTemplate $processedTemplate
    }

    # 4. Code sign
    if ($installer) {
        Invoke-CodeSign -InstallerPath $installer.FullName
    }

    Write-Step "=== Packaging complete ==="
    if ($installer) {
        Write-Step "Installer: $($installer.FullName)"
    } else {
        Write-Step "Output: $TargetDir\bundle\"
    }
}

Main
