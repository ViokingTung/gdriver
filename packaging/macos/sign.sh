#!/bin/bash
#
# gDriver macOS Code Signing Script
#
# Usage: ./sign.sh <path/to/gDriver.app>
#
# Signs all binaries inside the .app bundle in the correct order:
#   1. Extension binaries (innermost first)
#   2. Extension .appex bundles
#   3. Main app daemon
#   4. Main .app bundle
#
# Prerequisites:
#   - Apple Developer ID Application certificate in Keychain
#   - APPLE_TEAM_ID env var set (or TEAM_ID in sign-config.sh)
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENTITLEMENTS_DIR="${SCRIPT_DIR}/entitlements"

# ── Configuration ────────────────────────────────────────────────────────
APP_PATH="${1:-}"

# Load signing config if available
if [ -f "${SCRIPT_DIR}/sign-config.sh" ]; then
    source "${SCRIPT_DIR}/sign-config.sh"
fi

TEAM_ID="${APPLE_TEAM_ID:-${TEAM_ID:-}}"
SIGNING_IDENTITY="${SIGNING_IDENTITY:-Developer ID Application: ${TEAM_ID}}"

if [ -z "${APP_PATH}" ]; then
    echo "Usage: $0 <path/to/gDriver.app>"
    exit 1
fi

if [ ! -d "${APP_PATH}" ]; then
    echo "Error: ${APP_PATH} does not exist."
    exit 1
fi

if [ -z "${TEAM_ID}" ]; then
    echo "Error: TEAM_ID not set. Set APPLE_TEAM_ID env var."
    exit 1
fi

APP_CONTENTS="${APP_PATH}/Contents"
APP_MACOS="${APP_CONTENTS}/MacOS"
APP_PLUGINS="${APP_CONTENTS}/PlugIns"
FINDER_APPEX="${APP_PLUGINS}/GDriverFinderSync.appex"
FP_APPEX="${APP_PLUGINS}/GDriverFileProvider.appex"

# ── Colors ───────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; RED='\033[0;31m'; YELLOW='\033[1;33m'; NC='\033[0m'
log()  { echo -e "${GREEN}[sign]${NC} $*"; }
warn() { echo -e "${YELLOW}[sign]${NC} $*"; }

log "Signing: ${APP_PATH}"
log "Identity: ${SIGNING_IDENTITY}"
log "Team: ${TEAM_ID}"

# ── Signing flags ───────────────────────────────────────────────────────
CODESIGN_FLAGS="--force --verify --verbose --timestamp --options runtime"
CODESIGN_SIGN="--sign ${SIGNING_IDENTITY}"

# ── Step 1: Sign extension binaries ─────────────────────────────────────
sign_extension_binaries() {
    local appex_path="$1"
    local appex_name
    appex_name=$(basename "${appex_path}")

    log "Signing binaries in ${appex_name}..."

    # Sign Swift runtime dylibs if they exist
    if [ -d "${appex_path}/Contents/Frameworks" ]; then
        find "${appex_path}/Contents/Frameworks" -name "*.dylib" -type f | while read -r dylib; do
            log "  ${dylib}"
            codesign ${CODESIGN_FLAGS} ${CODESIGN_SIGN} "${dylib}"
        done
    fi

    # Sign the extension's main binary
    local ext_binary="${appex_path}/Contents/MacOS/"*
    if [ -f "${ext_binary}" ]; then
        log "  ${ext_binary}"
        codesign ${CODESIGN_FLAGS} ${CODESIGN_SIGN} "${ext_binary}"
    fi
}

# ── Step 2: Sign extension .appex bundles ────────────────────────────────
sign_appex_bundle() {
    local appex_path="$1"
    local appex_name
    appex_name=$(basename "${appex_path}")
    local entitlements_file="$2"

    log "Signing ${appex_name} bundle..."
    codesign ${CODESIGN_FLAGS} ${CODESIGN_SIGN} \
        --entitlements "${entitlements_file}" \
        "${appex_path}"
}

# ── Step 3: Sign daemon binary ──────────────────────────────────────────
sign_daemon() {
    log "Signing gdriver-daemon..."
    codesign ${CODESIGN_FLAGS} ${CODESIGN_SIGN} \
        --entitlements "${ENTITLEMENTS_DIR}/gdriver.entitlements" \
        "${APP_MACOS}/gdriver-daemon"
}

# ── Step 4: Sign main .app bundle ────────────────────────────────────────
sign_main_app() {
    log "Signing main application bundle..."
    codesign ${CODESIGN_FLAGS} ${CODESIGN_SIGN} \
        --entitlements "${ENTITLEMENTS_DIR}/gdriver.entitlements" \
        --deep \
        "${APP_PATH}"
}

# ── Verify ───────────────────────────────────────────────────────────────
verify_signing() {
    log "Verifying code signatures..."

    local all_ok=true

    verify_one() {
        local path="$1"
        if codesign --verify --deep --strict --verbose=2 "${path}" 2>&1; then
            log "  OK  ${path}"
        else
            warn "  FAIL  ${path}"
            all_ok=false
        fi
    }

    # Verify daemon
    verify_one "${APP_MACOS}/gdriver-daemon"

    # Verify extensions
    if [ -d "${FINDER_APPEX}" ]; then
        verify_one "${FINDER_APPEX}"
    fi
    if [ -d "${FP_APPEX}" ]; then
        verify_one "${FP_APPEX}"
    fi

    # Verify main app
    verify_one "${APP_PATH}"

    if [ "${all_ok}" = true ]; then
        log "All signatures verified."
    else
        warn "Some signatures failed verification."
    fi
}

# ── Main ─────────────────────────────────────────────────────────────────
log "Step 1: Signing extension binaries..."

if [ -d "${FINDER_APPEX}" ]; then
    sign_extension_binaries "${FINDER_APPEX}"
fi
if [ -d "${FP_APPEX}" ]; then
    sign_extension_binaries "${FP_APPEX}"
fi

log "Step 2: Signing extension bundles..."
if [ -d "${FINDER_APPEX}" ]; then
    sign_appex_bundle "${FINDER_APPEX}" "${ENTITLEMENTS_DIR}/findersync.entitlements"
fi
if [ -d "${FP_APPEX}" ]; then
    sign_appex_bundle "${FP_APPEX}" "${ENTITLEMENTS_DIR}/fileprovider.entitlements"
fi

log "Step 3: Signing daemon..."
sign_daemon

log "Step 4: Signing main application..."
sign_main_app

log "Step 5: Verification..."
verify_signing

log "=== Signing complete ==="
