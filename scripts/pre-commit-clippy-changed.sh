#!/usr/bin/env bash
# Pre-commit clippy: lint only the crates owning the staged files.
#
# Why: `cargo clippy --workspace --all-targets -- -D warnings` lints every
# workspace member on EVERY commit. That forces "commit only at
# workspace-green points" — a big refactor cannot land as a series of small
# atomic commits because an as-yet-untouched crate three layers away may be
# red mid-refactor. The coarse-commit pain (spec D3 / §7 F).
#
# This mirrors the per-crate strategy of `pre-commit-fmt-check.sh`: walk each
# staged file up to its owning crate's `Cargo.toml`, collect unique package
# names, and lint only those (`cargo clippy -p <name> --all-targets -- -D
# warnings`). Workspace `clippy.toml` / `[workspace.lints]` are honored
# because cargo picks them up from the workspace root regardless of which
# packages are selected.
#
# Authoritative full-workspace clippy now runs at `pre-push` (CI clippy
# parity); CI's `clippy` required job remains the final gate. This script is
# fast-feedback only.
set -euo pipefail

if [[ $# -eq 0 ]]; then
  exit 0
fi

declare -A seen=()
pkg_args=()
# Standalone manifests (anything carrying its own `[workspace]` table — fuzz
# crates, for instance) live outside the main workspace and must be linted
# via `--manifest-path`, since `cargo clippy -p <name>` from the workspace
# root cannot see them.
standalone_manifests=()

for f in "$@"; do
  # Lefthook's `glob: "**/*.rs"` already filters, but be defensive in case
  # the script is invoked manually with a mixed file list.
  [[ "$f" == *.rs ]] || continue

  d="$(dirname "$f")"
  while [[ "$d" != "." && "$d" != "/" ]]; do
    if [[ -f "$d/Cargo.toml" ]] && grep -q '^\[package\]' "$d/Cargo.toml"; then
      break
    fi
    d="$(dirname "$d")"
  done

  if [[ ! -f "$d/Cargo.toml" ]] || ! grep -q '^\[package\]' "$d/Cargo.toml"; then
    continue
  fi

  # Scope to the [package] table — a `name = "…"` under another table
  # (e.g. [[bin]], [package.metadata]) must not be mistaken for the crate.
  name="$(awk -F'"' '/^\[package\]/{p=1;next} /^\[/{p=0} p&&/^name[[:space:]]*=[[:space:]]*"/{print $2;exit}' "$d/Cargo.toml")"
  [[ -z "$name" ]] && continue

  if [[ -n "${seen[$name]:-}" ]]; then
    continue
  fi
  seen[$name]=1

  if grep -q '^\[workspace\]' "$d/Cargo.toml"; then
    standalone_manifests+=("$d/Cargo.toml")
  else
    pkg_args+=("-p" "$name")
  fi
done

if [[ ${#pkg_args[@]} -eq 0 && ${#standalone_manifests[@]} -eq 0 ]]; then
  exit 0
fi

# Print which crates we're linting (lefthook suppresses stdout on success;
# only the failure path surfaces this).
if [[ ${#pkg_args[@]} -gt 0 ]]; then
  echo "clippy (per-crate):" "${pkg_args[@]}"
  cargo clippy "${pkg_args[@]}" --all-targets -q -- -D warnings
fi

# Run each standalone manifest in its own invocation — `--manifest-path`
# accepts only one value, so we loop instead of batching.
for manifest in "${standalone_manifests[@]}"; do
  echo "clippy (standalone): $manifest"
  cargo clippy --manifest-path "$manifest" --all-targets -q -- -D warnings
done
