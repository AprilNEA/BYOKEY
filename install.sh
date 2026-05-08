#!/usr/bin/env sh
# byokey installer — downloads a release binary from GitHub.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/AprilNEA/BYOKEY/master/install.sh | sh
#
# Environment overrides:
#   BYOKEY_VERSION       Tag to install (default: latest release, e.g. v1.2.0).
#   BYOKEY_INSTALL_DIR   Where to install the binary (default: $HOME/.byokey/bin).

set -eu

REPO="AprilNEA/BYOKEY"
INSTALL_DIR="${BYOKEY_INSTALL_DIR:-$HOME/.byokey/bin}"

err() { printf 'error: %s\n' "$1" >&2; exit 1; }
info() { printf '%s\n' "$1"; }

# --- Detect platform ---------------------------------------------------------
case "$(uname -s)" in
  Linux)  os="unknown-linux-gnu" ;;
  Darwin) os="apple-darwin" ;;
  *) err "Unsupported OS: $(uname -s). Use Homebrew or 'cargo install byokey'." ;;
esac

case "$(uname -m)" in
  x86_64|amd64)   arch="x86_64" ;;
  aarch64|arm64)  arch="aarch64" ;;
  *) err "Unsupported architecture: $(uname -m)." ;;
esac

target="${arch}-${os}"

# --- Resolve version ---------------------------------------------------------
version="${BYOKEY_VERSION:-}"
if [ -z "$version" ]; then
  info "Resolving latest release..."
  version=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
    | sed -n -E '/"tag_name":/{ s/.*"tag_name": *"([^"]+)".*/\1/p; q; }')
  [ -n "$version" ] || err "Could not determine latest release tag."
fi

archive="byokey-${version}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/${version}/${archive}"

# --- Download & install ------------------------------------------------------
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

info "Downloading $url"
if ! curl -fsSL "$url" -o "$tmp/$archive"; then
  err "Download failed. Check that ${version} has a build for ${target} at https://github.com/${REPO}/releases"
fi

tar -xzf "$tmp/$archive" -C "$tmp"
[ -f "$tmp/byokey" ] || err "Archive did not contain a 'byokey' binary."

mkdir -p "$INSTALL_DIR"
mv "$tmp/byokey" "$INSTALL_DIR/byokey"
chmod +x "$INSTALL_DIR/byokey"

info ""
info "Installed byokey ${version} to ${INSTALL_DIR}/byokey"

# --- PATH hint ---------------------------------------------------------------
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*)
    info "Run: byokey --help"
    ;;
  *)
    info ""
    info "Add to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
    info "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    info ""
    info "Then run: byokey --help"
    ;;
esac
