#!/bin/bash
#
# gDriver macOS Packaging Build Script
#
# Usage: ./build.sh [--skip-sign] [--skip-notarize] [--arch <x86_64|arm64|universal>]
#
# Produces:
#   gDriver.app            — signed, notarized application bundle
#   gDriver_0.1.0.dmg      — distributable disk image
#
# Prerequisites:
#   - macOS 12+ with Xcode 15+
#   - Rust toolchain (both x86_64-apple-darwin and aarch64-apple-darwin targets)
#   - Node.js + pnpm
#   - Tauri CLI: cargo install tauri-cli
#   - Apple Developer account (for signing/notarization)
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
TAURI_DIR="${PROJECT_ROOT}/apps/gdriver-app"
TAURI_SRC="${TAURI_DIR}/src-tauri"
EXTENSIONS_DIR="${PROJECT_ROOT}/extensions"
ENTITLEMENTS_DIR="${SCRIPT_DIR}/entitlements"

# ── Configuration ────────────────────────────────────────────────────────
APP_NAME="gDriver"
BUNDLE_ID="io.gdriver.app"
APP_GROUP="io.gdriver.app.group"
TEAM_ID="${APPLE_TEAM_ID:-}"           # Set via env or signing config
APPLE_ID="${APPLE_ID:-}"               # Apple ID for notarization
APP_PASSWORD="${APPLE_APP_PASSWORD:-}" # App-specific password

VERSION="0.1.0"
ARCH="${ARCH:-universal}"              # x86_64 | arm64 | universal

SKIP_SIGN=false
SKIP_NOTARIZE=false
SKIP_BUILD=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-sign)     SKIP_SIGN=true; shift ;;
        --skip-notarize) SKIP_NOTARIZE=true; shift ;;
        --skip-build)    SKIP_BUILD=true; shift ;;
        --arch)          ARCH="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# ── Colors ───────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

log()  { echo -e "${GREEN}[gDriver]${NC} $*"; }
warn() { echo -e "${YELLOW}[gDriver] WARN${NC} $*"; }
err()  { echo -e "${RED}[gDriver] ERR${NC} $*"; }
step() { echo -e "${CYAN}[gDriver]${NC} === $* ==="; }

# ── Build daemon (universal binary) ─────────────────────────────────────
build_daemon() {
    step "Building gdriver-daemon (arch: ${ARCH})"

    cd "${PROJECT_ROOT}"

    if [ "${ARCH}" = "universal" ]; then
        log "Building for x86_64..."
        cargo build -p gdriver-daemon --release --target x86_64-apple-darwin

        log "Building for arm64..."
        cargo build -p gdriver-daemon --release --target aarch64-apple-darwin

        log "Creating universal binary with lipo..."
        mkdir -p target/release
        lipo -create \
            target/x86_64-apple-darwin/release/gdriver-daemon \
            target/aarch64-apple-darwin/release/gdriver-daemon \
            -output target/release/gdriver-daemon

        log "Universal daemon: target/release/gdriver-daemon"
        file target/release/gdriver-daemon
    elif [ "${ARCH}" = "x86_64" ]; then
        cargo build -p gdriver-daemon --release --target x86_64-apple-darwin
        cp target/x86_64-apple-darwin/release/gdriver-daemon target/release/gdriver-daemon
    else
        cargo build -p gdriver-daemon --release --target aarch64-apple-darwin
        cp target/aarch64-apple-darwin/release/gdriver-daemon target/release/gdriver-daemon
    fi
}

# ── Build Swift extensions ──────────────────────────────────────────────
build_extensions() {
    step "Building Finder Sync Extension"

    local finder_sync_dir="${EXTENSIONS_DIR}/findersync"
    cd "${finder_sync_dir}"
    swift build -c release \
        --arch x86_64 --arch arm64 \
        --package-path "${finder_sync_dir}" \
        -Xswiftc "-application-extension"

    log "  -> ${finder_sync_dir}/.build/release/GDriverFinderSync"

    step "Building FileProvider Extension"

    local file_provider_dir="${EXTENSIONS_DIR}/fileprovider"
    cd "${file_provider_dir}"
    swift build -c release \
        --arch x86_64 --arch arm64 \
        --package-path "${file_provider_dir}" \
        -Xswiftc "-application-extension"

    log "  -> ${file_provider_dir}/.build/release/GDriverFileProvider"
}

# ── Create extension .appex bundles ──────────────────────────────────────
create_appex_bundles() {
    local app_contents="$1"

    step "Creating Finder Sync .appex bundle"

    local finder_appex="${app_contents}/PlugIns/GDriverFinderSync.appex"
    mkdir -p "${finder_appex}/Contents/MacOS"

    cp "${EXTENSIONS_DIR}/findersync/.build/release/GDriverFinderSync" \
       "${finder_appex}/Contents/MacOS/GDriverFinderSync"

    # Generate Finder Sync Info.plist
    cat > "${finder_appex}/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>gDriver Finder Sync</string>
    <key>CFBundleExecutable</key>
    <string>GDriverFinderSync</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}.findersync</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>GDriverFinderSync</string>
    <key>CFBundlePackageType</key>
    <string>XPC!</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>NSExtension</key>
    <dict>
        <key>NSExtensionPointIdentifier</key>
        <string>com.apple.FinderSync</string>
        <key>NSExtensionPrincipalClass</key>
        <string>GDriverFinderSync.GDriverFinderSync</string>
    </dict>
    <key>NSExtensionFileProviderDocumentGroup</key>
    <string>${APP_GROUP}</string>
</dict>
</plist>
PLIST

    log "  -> ${finder_appex}"

    step "Creating FileProvider .appex bundle"

    local fp_appex="${app_contents}/PlugIns/GDriverFileProvider.appex"
    mkdir -p "${fp_appex}/Contents/MacOS"

    cp "${EXTENSIONS_DIR}/fileprovider/.build/release/GDriverFileProvider" \
       "${fp_appex}/Contents/MacOS/GDriverFileProvider"

    # Generate FileProvider Info.plist
    cat > "${fp_appex}/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>gDriver FileProvider</string>
    <key>CFBundleExecutable</key>
    <string>GDriverFileProvider</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}.fileprovider</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>GDriverFileProvider</string>
    <key>CFBundlePackageType</key>
    <string>XPC!</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>NSExtension</key>
    <dict>
        <key>NSExtensionFileProviderSupportsEnumeration</key>
        <true/>
        <key>NSExtensionPointIdentifier</key>
        <string>com.apple.fileprovider-nonui</string>
        <key>NSExtensionPrincipalClass</key>
        <string>GDriverFileProvider.FileProviderExtension</string>
    </dict>
    <key>NSExtensionFileProviderDocumentGroup</key>
    <string>${APP_GROUP}</string>
</dict>
</plist>
PLIST

    log "  -> ${fp_appex}"
}

# ── Build Tauri app ─────────────────────────────────────────────────────
build_tauri_app() {
    step "Building Tauri desktop app (arch: ${ARCH})"

    cd "${TAURI_DIR}"

    if [ ! -d "node_modules" ]; then
        pnpm install
    fi

    # Set Rust target based on arch
    if [ "${ARCH}" = "universal" ]; then
        export TAURI_BUNDLE_UNIVERSAL="true"
    elif [ "${ARCH}" = "x86_64" ]; then
        export CARGO_BUILD_TARGET="x86_64-apple-darwin"
    elif [ "${ARCH}" = "arm64" ]; then
        export CARGO_BUILD_TARGET="aarch64-apple-darwin"
    fi

    cargo tauri build --bundles dmg

    log "Tauri build complete."
}

# ── Embed daemon and extensions into .app ───────────────────────────────
embed_components() {
    step "Embedding daemon and extensions into .app bundle"

    # Find the generated .app
    local app_path
    app_path=$(ls -dt "${TAURI_SRC}/target/release/bundle/macos/"*.app 2>/dev/null | head -1)

    if [ -z "${app_path}" ]; then
        # Tauri v2 might put it elsewhere
        app_path=$(ls -dt "${TAURI_SRC}/target/release/bundle/dmg/"*.app 2>/dev/null | head -1)
    fi

    if [ -z "${app_path}" ]; then
        # Check broader search
        app_path=$(find "${PROJECT_ROOT}/target" -name "gDriver.app" -maxdepth 5 -type d 2>/dev/null | head -1)
    fi

    if [ -z "${app_path}" ]; then
        err "Could not find gDriver.app. Tauri build may have failed."
        exit 1
    fi

    log "App bundle: ${app_path}"

    local app_contents="${app_path}/Contents"
    local app_macos="${app_contents}/MacOS"

    # ── Embed daemon ──
    log "Embedding gdriver-daemon..."
    cp "${PROJECT_ROOT}/target/release/gdriver-daemon" "${app_macos}/gdriver-daemon"
    chmod 755 "${app_macos}/gdriver-daemon"

    # ── Create PlugIns directory and embed extensions ──
    create_appex_bundles "${app_contents}"

    # ── Update main app Info.plist ──
    log "Updating main app Info.plist..."
    /usr/libexec/PlistBuddy -c "Add :NSExtensionFileProviderDocumentGroup string ${APP_GROUP}" \
        "${app_contents}/Info.plist" 2>/dev/null || \
    /usr/libexec/PlistBuddy -c "Set :NSExtensionFileProviderDocumentGroup ${APP_GROUP}" \
        "${app_contents}/Info.plist" 2>/dev/null || true

    log "App bundle structure:"
    find "${app_path}" -maxdepth 4 -type f -o -type d | sed "s|${app_path}|gDriver.app|" | sort

    echo "${app_path}"
}

# ── Code signing ────────────────────────────────────────────────────────
sign_app() {
    if [ "${SKIP_SIGN}" = true ]; then
        warn "Skipping code signing (--skip-sign)"
        return
    fi

    step "Code signing"

    if [ -z "${TEAM_ID}" ]; then
        warn "APPLE_TEAM_ID not set. Skipping signing."
        warn "Set APPLE_TEAM_ID env var or configure in packaging/macos/signing/sign-config.sh"
        return
    fi

    local sign_script="${SCRIPT_DIR}/sign.sh"
    if [ -x "${sign_script}" ]; then
        bash "${sign_script}" "${app_path}"
    else
        warn "sign.sh not found. Run manual signing with:"
        echo "  codesign --deep --force --verify --verbose \\"
        echo "    --sign 'Developer ID Application: \${TEAM_ID}' \\"
        echo "    --entitlements ${ENTITLEMENTS_DIR}/gdriver.entitlements \\"
        echo "    --options runtime \\"
        echo "    \${app_path}"
    fi
}

# ── Notarization ────────────────────────────────────────────────────────
notarize_app() {
    if [ "${SKIP_NOTARIZE}" = true ]; then
        warn "Skipping notarization (--skip-notarize)"
        return
    fi

    step "Notarization"

    if [ -z "${APPLE_ID}" ] || [ -z "${APP_PASSWORD}" ]; then
        warn "APPLE_ID or APPLE_APP_PASSWORD not set. Skipping notarization."
        return
    fi

    local notarize_script="${SCRIPT_DIR}/notarize.sh"
    if [ -x "${notarize_script}" ]; then
        bash "${notarize_script}" "${app_path}"
    else
        warn "notarize.sh not found. Run manual notarization."
    fi
}

# ── Main ─────────────────────────────────────────────────────────────────
main() {
    step "=== gDriver macOS Packaging ==="
    log "Architecture : ${ARCH}"
    log "Signing      : $([ "${SKIP_SIGN}" = true ] && echo 'SKIPPED' || echo 'enabled')"
    log "Notarization : $([ "${SKIP_NOTARIZE}" = true ] && echo 'SKIPPED' || echo 'enabled')"
    log "Project root : ${PROJECT_ROOT}"
    echo ""

    if [ "${SKIP_BUILD}" = true ]; then
        warn "Skipping build steps (--skip-build)"
    else
        # 1. Build daemon (universal or single arch)
        build_daemon

        # 2. Build Swift extensions
        build_extensions

        # 3. Build Tauri app (generates .app and .dmg)
        build_tauri_app
    fi

    # 4. Embed daemon + extensions into .app bundle
    local app_path
    app_path=$(embed_components)

    # 5. Re-create .dmg with the updated .app
    step "Re-creating DMG with embedded components"
    local dmg_output="${PROJECT_ROOT}/target/release/bundle/macos/${APP_NAME}_${VERSION}_${ARCH}.dmg"
    mkdir -p "$(dirname "${dmg_output}")"

    # Remove old DMG if exists
    rm -f "${dmg_output}"

    # Create DMG
    hdiutil create -volname "${APP_NAME}" \
        -srcfolder "${app_path}" \
        -ov -format UDBZ \
        "${dmg_output}"

    log "DMG created: ${dmg_output}"

    # 6. Sign the DMG
    if [ "${SKIP_SIGN}" != true ] && [ -n "${TEAM_ID}" ]; then
        log "Signing DMG..."
        codesign --force --verify --verbose \
            --sign "Developer ID Application: ${TEAM_ID}" \
            "${dmg_output}"
    fi

    # 7. Notarize the DMG
    if [ "${SKIP_NOTARIZE}" != true ] && [ -n "${APPLE_ID}" ]; then
        notarize_app "${dmg_output}"
    fi

    # 8. Generate SHA-256 checksum
    step "Generating SHA-256 checksum"
    shasum -a 256 "${dmg_output}" | tee "${dmg_output}.sha256"

    step "=== Packaging complete ==="
    log "App  : ${app_path}"
    log "DMG  : ${dmg_output}"
}

main
