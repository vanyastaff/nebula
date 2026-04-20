#!/usr/bin/env bash
# Install Nebula dev tools via cargo-binstall (prebuilt binaries — much faster
# than building from source). Falls back to `cargo install` when no prebuilt
# is available for the host platform.
#
# Prerequisites: rustup (any toolchain — Cargo.toml `workspace.package.rust-version`
# pins MSRV at 1.95, CI mirrors that). Run `rustup default stable` once.
#
# Usage: bash scripts/install-tools.sh
#
# Re-running is safe: cargo-binstall skips tools already at the requested version.

set -euo pipefail

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found. Install rustup first: https://rustup.rs/" >&2
  exit 1
fi

if ! command -v cargo-binstall >/dev/null 2>&1; then
  echo "→ bootstrapping cargo-binstall (one-time)"
  cargo install cargo-binstall --locked
fi

# Tool list mirrors what CI / lefthook need.
TOOLS=(
  "taplo-cli@0.9.3"           # TOML formatter (pre-commit)
  "typos-cli@1.27"            # spell check (pre-commit)
  "cargo-nextest@0.9"         # test runner
  "cargo-deny@0.16"           # license / advisory check
  "cargo-shear@1.1"           # unused dep detector
  "cargo-semver-checks@0.40"  # semver compliance
  "cargo-audit@0.21"          # security advisory check
  "cargo-release@0.25"        # release tooling
  "sccache@0.8"               # shared compile cache
  "convco@0.6"                # conventional commit lint (commit-msg hook)
)

install_one() {
  local spec="$1"
  echo "→ ${spec}"
  if cargo binstall --no-confirm --quiet "$spec"; then
    return 0
  fi
  echo "  binstall failed, falling back to cargo install (may take several minutes)"
  cargo install --locked "${spec%@*}" --version "${spec#*@}"
}

for spec in "${TOOLS[@]}"; do
  install_one "$spec"
done

echo
echo "✓ Done. Quick check:"
echo "  rustc --version"
echo "  cargo nextest --version"
echo "  taplo --version"
