#!/bin/bash
#
# gDriver macOS Notarization Script
#
# Usage: ./notarize.sh <path/to/gDriver.dmg>     (for DMG)
#        ./notarize.sh <path/to/gDriver.app>     (for .app bundle, auto-zips)
#
# Submits the artifact to Apple Notary Service and staples the ticket.
#
# Prerequisites:
#   - Apple Developer account
#   - APPLE_ID and APPLE_APP_PASSWORD env vars (app-specific password)
#   - Or: set in packaging/macos/sign-config.sh
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARTIFACT_PATH="${1:-}"

# Load config if available
if [ -f "${SCRIPT_DIR}/sign-config.sh" ]; then
    source "${SCRIPT_DIR}/sign-config.sh"
fi

APPLE_ID="${APPLE_ID:-}"
APPLE_PASSWORD="${APPLE_APP_PASSWORD:-}"
TEAM_ID="${APPLE_TEAM_ID:-}"

if [ -z "${ARTIFACT_PATH}" ]; then
    echo "Usage: $0 <path/to/gDriver.dmg|path/to/gDriver.app>"
    exit 1
fi

if [ ! -e "${ARTIFACT_PATH}" ]; then
    echo "Error: ${ARTIFACT_PATH} does not exist."
    exit 1
fi

if [ -z "${APPLE_ID}" ]; then
    echo "Error: APPLE_ID not set."
    echo "  export APPLE_ID=your@email.com"
    exit 1
fi

if [ -z "${APPLE_PASSWORD}" ]; then
    echo "Error: APPLE_APP_PASSWORD not set."
    echo "  Create one at https://appleid.apple.com/account/manage"
    echo "  export APPLE_APP_PASSWORD=xxxx-xxxx-xxxx-xxxx"
    exit 1
fi

# ── Colors ───────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; RED='\033[0;31m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
log()  { echo -e "${GREEN}[notarize]${NC} $*"; }
warn() { echo -e "${YELLOW}[notarize]${NC} $*"; }
info() { echo -e "${CYAN}[notarize]${NC} $*"; }

# ── Determine artifact type and prepare for submission ───────────────────
SUBMISSION_PATH="${ARTIFACT_PATH}"
ZIP_CREATED=false

prepare_submission() {
    # If it's an .app bundle, zip it
    if [ -d "${ARTIFACT_PATH}" ] && [[ "${ARTIFACT_PATH}" == *.app ]]; then
        local app_name
        app_name=$(basename "${ARTIFACT_PATH}")
        SUBMISSION_PATH="/tmp/${app_name%.app}_notarize.zip"
        log "Zipping .app for submission..."
        /usr/bin/ditto -c -k --keepParent "${ARTIFACT_PATH}" "${SUBMISSION_PATH}"
        ZIP_CREATED=true
        log "  -> ${SUBMISSION_PATH}"
    fi
}

cleanup_zip() {
    if [ "${ZIP_CREATED}" = true ] && [ -f "${SUBMISSION_PATH}" ]; then
        rm -f "${SUBMISSION_PATH}"
    fi
}

# ── Submit for notarization ──────────────────────────────────────────────
submit_for_notarization() {
    log "Submitting to Apple Notary Service..."
    info "This may take several minutes..."

    local team_arg=""
    if [ -n "${TEAM_ID}" ]; then
        team_arg="--team-id ${TEAM_ID}"
    fi

    local result
    result=$(xcrun notarytool submit "${SUBMISSION_PATH}" \
        --apple-id "${APPLE_ID}" \
        --password "${APPLE_PASSWORD}" \
        ${team_arg} \
        --wait \
        --output-format json 2>&1)

    echo "${result}" | python3 -m json.tool 2>/dev/null || echo "${result}"

    # Extract submission ID
    local submission_id
    submission_id=$(echo "${result}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('id',''))" 2>/dev/null || echo "")

    # Check status
    local status
    status=$(echo "${result}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('status',''))" 2>/dev/null || echo "")

    if [ "${status}" = "Accepted" ]; then
        log "Notarization: ACCEPTED"
        echo "${submission_id}"
    elif [ "${status}" = "Invalid" ]; then
        warn "Notarization: INVALID"
        # Get detailed log
        if [ -n "${submission_id}" ]; then
            info "Fetching detailed log..."
            xcrun notarytool log "${submission_id}" \
                --apple-id "${APPLE_ID}" \
                --password "${APPLE_PASSWORD}" \
                ${team_arg} 2>&1 || true
        fi
        exit 1
    else
        warn "Notarization status: ${status}"
        if [ -n "${submission_id}" ]; then
            info "Fetching detailed log..."
            xcrun notarytool log "${submission_id}" \
                --apple-id "${APPLE_ID}" \
                --password "${APPLE_PASSWORD}" \
                ${team_arg} 2>&1 || true
        fi
        exit 1
    fi
}

# ── Staple notarization ticket ───────────────────────────────────────────
staple_ticket() {
    log "Stapling notarization ticket..."

    if [ -d "${ARTIFACT_PATH}" ] && [[ "${ARTIFACT_PATH}" == *.app ]]; then
        xcrun stapler staple "${ARTIFACT_PATH}"
    else
        xcrun stapler staple "${ARTIFACT_PATH}"
    fi

    log "Ticket stapled."
}

# ── Verify notarization ──────────────────────────────────────────────────
verify_notarization() {
    log "Verifying notarization..."

    if spctl --assess --verbose --type install "${ARTIFACT_PATH}" 2>&1; then
        log "Gatekeeper check: PASSED"
    else
        warn "Gatekeeper check: FAILED"
    fi
}

# ── Main ─────────────────────────────────────────────────────────────────
log "=== gDriver macOS Notarization ==="
log "Artifact: ${ARTIFACT_PATH}"
log "Apple ID: ${APPLE_ID}"

prepare_submission

# Submit
submit_for_notarization

# Clean up temp zip if created
cleanup_zip

# Staple
staple_ticket

# Verify
verify_notarization

log "=== Notarization complete ==="
