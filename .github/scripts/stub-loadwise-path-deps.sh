#!/usr/bin/env bash
# Create dummy `loadwise` and `loadwise-core` crates at the path release-plz
# resolves to when checking out historical commits.
#
# Background: commits 8a17c8f..before-50c5227 contain
#   loadwise      = { path = "../../AprilNEA/loadwise/crates/loadwise" }
#   loadwise-core = { path = "../../AprilNEA/loadwise/crates/loadwise-core" }
# release-plz clones the repo into /tmp/.tmpXXXX/BYOKEY so the relative path
# resolves to /tmp/AprilNEA/loadwise/crates/<name>. Those directories don't
# exist in CI, so `cargo package` fails at the equality check.
#
# These stubs let cargo load the historical manifest without actually
# building loadwise — release-plz only reads the packaged file list for the
# target crates (byokey-config, byokey-provider, etc.), not the path-deps.
set -euo pipefail

STUB_ROOT="/tmp/AprilNEA/loadwise/crates"

create_stub() {
    local name="$1"
    local version="$2"
    local dir="${STUB_ROOT}/${name}"
    mkdir -p "${dir}/src"
    cat > "${dir}/Cargo.toml" <<EOF
[package]
name = "${name}"
version = "${version}"
edition = "2021"
license = "MIT"
description = "CI stub — historical path-dep resolver for release-plz."
EOF
    cat > "${dir}/src/lib.rs" <<'EOF'
// CI stub. Intentionally empty — see .github/scripts/stub-loadwise-path-deps.sh
EOF
    echo "created stub ${dir}"
}

create_stub "loadwise" "0.1.0"
create_stub "loadwise-core" "0.1.0"
