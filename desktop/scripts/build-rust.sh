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

# ---------------------------------------------------------------------------
# Locate cargo. Xcode's shell inherits a minimal PATH, so we probe in order:
#   1. rustup default              ~/.cargo/bin
#   2. mise-managed rust           ~/.local/share/mise/installs/rust/**/bin
#   3. rtx-managed rust            ~/.local/share/rtx/installs/rust/**/bin
#   4. Homebrew rustup             /opt/homebrew/bin, /usr/local/bin
#   5. mise/rtx shim activation    try `mise exec` or `rtx exec`
# The user may also override by setting CARGO=/path/to/cargo in the Xcode
# build environment.
# ---------------------------------------------------------------------------

# Allow explicit override via env var.
CARGO="${CARGO:-}"

if [ -z "${CARGO}" ] && ! command -v cargo >/dev/null 2>&1; then
    # Build a candidate list of bin dirs to add to PATH.
    EXTRA_PATHS=(
        "$HOME/.cargo/bin"
        "/opt/homebrew/bin"
        "/usr/local/bin"
    )

    # Glob-expand mise rust installs: pick the newest (last sorted) bin dir.
    if [ -d "$HOME/.local/share/mise/installs/rust" ]; then
        while IFS= read -r -d '' d; do
            EXTRA_PATHS+=("$d")
        done < <(find "$HOME/.local/share/mise/installs/rust" -maxdepth 2 -type d -name bin -print0 2>/dev/null | sort -z)
    fi

    # Glob-expand rtx rust installs similarly.
    if [ -d "$HOME/.local/share/rtx/installs/rust" ]; then
        while IFS= read -r -d '' d; do
            EXTRA_PATHS+=("$d")
        done < <(find "$HOME/.local/share/rtx/installs/rust" -maxdepth 2 -type d -name bin -print0 2>/dev/null | sort -z)
    fi

    # Prepend all candidate dirs to PATH in one shot.
    EXTRA_PATH_STR="$(IFS=:; echo "${EXTRA_PATHS[*]}")"
    export PATH="${EXTRA_PATH_STR}:$PATH"
fi

# If still not found, try mise/rtx shim invocation.
if [ -z "${CARGO}" ] && ! command -v cargo >/dev/null 2>&1; then
    if command -v mise >/dev/null 2>&1; then
        # Verify mise can resolve cargo before committing.
        if mise exec -- cargo --version >/dev/null 2>&1; then
            CARGO="mise exec -- cargo"
        fi
    fi
fi

if [ -z "${CARGO}" ] && ! command -v cargo >/dev/null 2>&1; then
    if command -v rtx >/dev/null 2>&1; then
        if rtx exec -- cargo --version >/dev/null 2>&1; then
            CARGO="rtx exec -- cargo"
        fi
    fi
fi

if [ -z "${CARGO}" ]; then
    if command -v cargo >/dev/null 2>&1; then
        CARGO="cargo"
    else
        echo "error: cargo not found after probing common locations." >&2
        echo "  Install Rust via rustup (https://rustup.rs) or 'brew install rustup'," >&2
        echo "  or set the CARGO environment variable in Xcode build settings." >&2
        exit 1
    fi
fi

# Split CARGO into an array so multi-word shims ("mise exec -- cargo") and
# paths with spaces both work. Array-expansion preserves word boundaries.
read -r -a CARGO_CMD <<< "${CARGO}"

cd "${RUST_DIR}"

if [ "${CONFIGURATION}" = "Release" ]; then
    # -----------------------------------------------------------------------
    # Universal-binary guard for Release builds.
    #
    # When Xcode targets a universal build (ARCHS contains both arm64 and
    # x86_64) we must produce a fat binary via lipo. For single-arch Release
    # builds we fall through to the normal single-target path below.
    # -----------------------------------------------------------------------
    case "${ARCHS}" in
        *arm64*x86_64* | *x86_64*arm64*)
            ARM_TARGET="aarch64-apple-darwin"
            X86_TARGET="x86_64-apple-darwin"

            if command -v rustup >/dev/null 2>&1; then
                for T in "${ARM_TARGET}" "${X86_TARGET}"; do
                    if ! rustup target list --installed 2>/dev/null | grep -qx "${T}"; then
                        echo "note: rust target ${T} not installed — running 'rustup target add ${T}'"
                        rustup target add "${T}"
                    fi
                done
            else
                echo "warning: rustup not on PATH; assuming targets ${ARM_TARGET} and ${X86_TARGET} are pre-installed." >&2
                echo "         Install them via your toolchain manager if the build fails (e.g. 'mise exec -- rustup target add aarch64-apple-darwin x86_64-apple-darwin')." >&2
            fi

            echo "building byokey (release, universal: ${ARM_TARGET} + ${X86_TARGET})…"
            "${CARGO_CMD[@]}" build --release --target "${ARM_TARGET}" --bin byokey
            "${CARGO_CMD[@]}" build --release --target "${X86_TARGET}" --bin byokey

            ARM_BIN="${RUST_DIR}/target/${ARM_TARGET}/release/byokey"
            X86_BIN="${RUST_DIR}/target/${X86_TARGET}/release/byokey"
            FAT_BIN="${RUST_DIR}/target/universal-apple-darwin/release/byokey"
            mkdir -p "$(dirname "${FAT_BIN}")"
            lipo -create "${ARM_BIN}" "${X86_BIN}" -output "${FAT_BIN}"
            SRC="${FAT_BIN}"
            ;;
        *)
            echo "building byokey (release, ${RUST_TARGET})…"
            "${CARGO_CMD[@]}" build --release --target "${RUST_TARGET}" --bin byokey
            SRC="${RUST_DIR}/target/${RUST_TARGET}/release/byokey"
            ;;
    esac
else
    echo "building byokey (debug)…"
    "${CARGO_CMD[@]}" build --bin byokey
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
