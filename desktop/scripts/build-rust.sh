#!/bin/bash
# Build and embed the byokey Rust binary for the Byokey desktop app.
#
# Always invokes `cargo build` so Xcode picks up Rust source changes without
# the user having to run `make build` out-of-band. Cargo's incremental
# compilation makes no-op rebuilds fast (~100 ms), so Swift-only iteration
# isn't penalized.
set -euo pipefail

RUST_DIR="${PROJECT_DIR}/.."

# Map Xcode CONFIGURATION → Cargo profile.
case "${CONFIGURATION}" in
    Release)  RUST_PROFILE="release" ;;
    *)        RUST_PROFILE="debug"   ;;
esac

# Map Xcode ARCHS → Rust target triple. Release pins the triple for a
# deterministic output path; Debug uses cargo's default target dir to match
# `make build` output.
case "${ARCHS}" in
    *arm64*) RUST_TARGET="aarch64-apple-darwin" ;;
    *)       RUST_TARGET="x86_64-apple-darwin"  ;;
esac

# Locate cargo. Xcode's shell inherits a minimal PATH.
if ! command -v cargo >/dev/null 2>&1; then
    export PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found. Install rustup from https://rustup.rs or 'brew install rustup'." >&2
    exit 1
fi

cd "${RUST_DIR}"

if [ "${CONFIGURATION}" = "Release" ]; then
    echo "building byokey (release, ${RUST_TARGET})…"
    cargo build --release --target "${RUST_TARGET}" --bin byokey
    SRC="${RUST_DIR}/target/${RUST_TARGET}/${RUST_PROFILE}/byokey"
else
    echo "building byokey (debug)…"
    cargo build --bin byokey
    SRC="${RUST_DIR}/target/${RUST_PROFILE}/byokey"
fi

if [ ! -f "${SRC}" ]; then
    echo "error: cargo reported success but produced no binary at ${SRC}" >&2
    exit 1
fi

OUTPUT_DIR="${BUILT_PRODUCTS_DIR}/${CONTENTS_FOLDER_PATH}/Resources"
mkdir -p "${OUTPUT_DIR}"
DEST="${OUTPUT_DIR}/byokey"

# Only overwrite when the source is actually newer, to avoid forcing a re-sign
# on every Swift-only rebuild.
if [ ! -f "${DEST}" ] || [ "${SRC}" -nt "${DEST}" ]; then
    cp -f "${SRC}" "${DEST}"
    echo "byokey binary copied: ${SRC} → ${DEST}"
else
    echo "byokey binary up to date"
fi
