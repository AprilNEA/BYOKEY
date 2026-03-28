#!/bin/bash
set -euo pipefail
RUST_DIR="${PROJECT_DIR}/.."
case "${ARCHS}" in
    *arm64*) RUST_TARGET="aarch64-apple-darwin" ;;
    *)       RUST_TARGET="x86_64-apple-darwin" ;;
esac

if [ "${CONFIGURATION}" = "Release" ]; then
    RUST_PROFILE="release"
    export PATH="$HOME/.cargo/bin:$PATH"
    cd "${RUST_DIR}"
    cargo build --release --target "${RUST_TARGET}"
else
    RUST_PROFILE="debug"
    # Debug: use pre-built binary from 'make build', skip cargo build
    BINARY="${RUST_DIR}/target/${RUST_TARGET}/${RUST_PROFILE}/byokey"
    if [ ! -f "${BINARY}" ]; then
        BINARY="${RUST_DIR}/target/${RUST_PROFILE}/byokey"
    fi
    if [ ! -f "${BINARY}" ]; then
        echo "error: byokey binary not found. Run 'make build' first." >&2
        exit 1
    fi
fi

OUTPUT_DIR="${BUILT_PRODUCTS_DIR}/${CONTENTS_FOLDER_PATH}/Resources"
mkdir -p "${OUTPUT_DIR}"
cp -f "${RUST_DIR}/target/${RUST_TARGET}/${RUST_PROFILE}/byokey" "${OUTPUT_DIR}/byokey" 2>/dev/null \
    || cp -f "${RUST_DIR}/target/${RUST_PROFILE}/byokey" "${OUTPUT_DIR}/byokey"
