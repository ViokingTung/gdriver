#!/bin/bash
#
# gDriver Linux Packaging Build Helper
#
# Usage: ./build.sh [deb|rpm|appimage|all]
#
# This script:
#   1. Builds the daemon binary (release)
#   2. Builds the Tauri desktop app (release)
#   3. Injects maintainer scripts into .deb packages
#   4. Copies extensions into the install tree
#
# Prerequisites:
#   - Rust toolchain (stable)
#   - Node.js + pnpm
#   - Tauri CLI: cargo install tauri-cli
#   - Linux: dpkg-deb, rpm-build (for .deb/.rpm)
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PACKAGING_DIR="${SCRIPT_DIR}"
EXTENSIONS_DIR="${PROJECT_ROOT}/extensions"
DAEMON_CRATE="${PROJECT_ROOT}/crates/gdriver-daemon"

BUILD_MODE="${1:-all}"

# ── Colors ───────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log()  { echo -e "${GREEN}[gDriver]${NC} $*"; }
warn() { echo -e "${YELLOW}[gDriver]${NC} $*"; }
err()  { echo -e "${RED}[gDriver]${NC} $*"; }

# ── Build daemon ─────────────────────────────────────────────────────
build_daemon() {
    log "Building gdriver-daemon (release)..."
    cd "${PROJECT_ROOT}"
    cargo build -p gdriver-daemon --release
    log "Daemon built: target/release/gdriver-daemon"
}

# ── Build Tauri app ──────────────────────────────────────────────────
build_tauri() {
    log "Building gDriver Tauri app (release)..."
    cd "${PROJECT_ROOT}/apps/gdriver-app"

    # Install frontend dependencies if needed
    if [ ! -d "node_modules" ]; then
        pnpm install
    fi

    # Build with Tauri CLI
    if command -v cargo-tauri >/dev/null 2>&1; then
        cargo tauri build --bundles "${1}" 2>&1 || {
            warn "cargo tauri build failed; trying via cargo instead."
            cargo build --release
        }
    else
        warn "cargo-tauri not found; install with: cargo install tauri-cli"
        warn "Building only Rust binary; skip packaging with: npm run tauri build"
        cargo build --release
        return 1
    fi

    log "Tauri build complete."
}

# ── Inject maintainer scripts into .deb ───────────────────────────────
repack_deb() {
    log "Re-packaging .deb with maintainer scripts..."

    # Find the generated .deb
    BUNDLE_DIR="${PROJECT_ROOT}/target/release/bundle/deb"
    if [ ! -d "${BUNDLE_DIR}" ]; then
        # Tauri v2 puts bundles under src-tauri/target
        BUNDLE_DIR="${PROJECT_ROOT}/apps/gdriver-app/src-tauri/target/release/bundle/deb"
    fi

    ORIG_DEB=$(ls -t "${BUNDLE_DIR}"/*.deb 2>/dev/null | head -1)
    if [ -z "${ORIG_DEB}" ]; then
        err "No .deb found in ${BUNDLE_DIR}; skipping re-pack."
        return 1
    fi

    DEB_NAME=$(basename "${ORIG_DEB}")
    WORK_DIR="$(mktemp -d)"
    log "  Extracting ${DEB_NAME} → ${WORK_DIR}"

    # Extract the .deb
    dpkg-deb -R "${ORIG_DEB}" "${WORK_DIR}"

    # Inject maintainer scripts
    mkdir -p "${WORK_DIR}/DEBIAN"
    for script in postinst postrm; do
        if [ -f "${PACKAGING_DIR}/deb/${script}" ]; then
            cp "${PACKAGING_DIR}/deb/${script}" "${WORK_DIR}/DEBIAN/${script}"
            chmod 0755 "${WORK_DIR}/DEBIAN/${script}"
            log "  Injected ${script}"
        fi
    done

    # Stage extension files into the package
    EXT_STAGE="${WORK_DIR}/usr/share/gdriver/extensions"
    mkdir -p "${EXT_STAGE}/nautilus/icons" "${EXT_STAGE}/dolphin/icons"

    if [ -d "${EXTENSIONS_DIR}/nautilus" ]; then
        cp -r "${EXTENSIONS_DIR}/nautilus/"* "${EXT_STAGE}/nautilus/"
        log "  Staged Nautilus extension"
    fi
    if [ -d "${EXTENSIONS_DIR}/dolphin" ]; then
        cp -r "${EXTENSIONS_DIR}/dolphin/"* "${EXT_STAGE}/dolphin/"
        log "  Staged Dolphin extension"
    fi

    # Stage daemon binary
    DAEMON_BIN="${PROJECT_ROOT}/target/release/gdriver-daemon"
    if [ -f "${DAEMON_BIN}" ]; then
        mkdir -p "${WORK_DIR}/usr/bin"
        cp "${DAEMON_BIN}" "${WORK_DIR}/usr/bin/gdriver-daemon"
        chmod 0755 "${WORK_DIR}/usr/bin/gdriver-daemon"
        log "  Staged gdriver-daemon"
    fi

    # Re-pack
    OUTPUT_DEB="${BUNDLE_DIR}/${DEB_NAME%.deb}-repacked.deb"
    dpkg-deb -b "${WORK_DIR}" "${OUTPUT_DEB}"
    rm -rf "${WORK_DIR}"

    log "Repacked .deb: ${OUTPUT_DEB}"
}

# ── Main ─────────────────────────────────────────────────────────────
main() {
    log "=== gDriver Linux Packaging ==="
    log "Project root: ${PROJECT_ROOT}"

    # Build daemon first
    build_daemon

    case "${BUILD_MODE}" in
        deb)
            build_tauri "deb" || true
            repack_deb
            ;;
        rpm)
            build_tauri "rpm" || true
            # RPM scripts are handled via the spec file; manual repack
            # is similar but uses rpmrebuild or a custom spec.
            warn "RPM re-packaging: install rpmrebuild or modify the generated .spec."
            warn "  rpmrebuild --add-file=packaging/linux/rpm/post:%post \\"
            warn "             --add-file=packaging/linux/rpm/postun:%postun \\"
            warn "             -p target/release/bundle/rpm/*.rpm"
            ;;
        appimage)
            build_tauri "appimage" || true
            log "AppImage generated by Tauri bundler (includes daemon via bundle config)."
            ;;
        all)
            build_tauri "deb" || true
            repack_deb
            build_tauri "rpm" || true
            build_tauri "appimage" || true
            ;;
        *)
            err "Unknown target: ${BUILD_MODE}"
            err "Usage: $0 [deb|rpm|appimage|all]"
            exit 1
            ;;
    esac

    log "=== Packaging complete ==="
    log "Output: ${PROJECT_ROOT}/target/release/bundle/"
}

main
