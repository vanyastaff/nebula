#!/usr/bin/env bash
# Pre-commit fmt-check: format only the crates owning the staged files.
#
# Why: `cargo +nightly fmt --all -- --check` builds a long internal command
# line iterating every workspace member. On Windows with deep working-tree
# paths (e.g. `C:\Users\<user>\...\.worktrees\nebula\<branch>\`), that line
# exceeds the ~32k cmdline limit and cargo fails with `OS error 206`.
#
# This script mirrors the per-crate strategy used by
# `pre-push-crate-diff.sh`: walk each staged file up to its owning crate's
# `Cargo.toml`, collect unique `nebula-*` package names, and pass them as
# `-p` flags. Workspace `rustfmt.toml` is honored because cargo-fmt picks
# it up from the workspace root regardless of which packages are selected.
#
# CI fmt-check on Linux (`cargo +nightly fmt --all -- --check`) remains the
# authoritative gate; this script is fast-feedback only.
set -euo pipefail

if [[ $# -eq 0 ]]; then
  exit 0
fi

declare -A seen=()
pkg_args=()

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

  name="$(awk -F'"' '/^name[[:space:]]*=[[:space:]]*"/ { print $2; exit }' "$d/Cargo.toml")"
  [[ -z "$name" ]] && continue

  if [[ -z "${seen[$name]:-}" ]]; then
    seen[$name]=1
    pkg_args+=("-p" "$name")
  fi
done

if [[ ${#pkg_args[@]} -eq 0 ]]; then
  exit 0
fi

# Print which crates we're checking (lefthook suppresses stdout on success;
# only the failure path surfaces this).
echo "fmt-check (per-crate):" "${pkg_args[@]}"
exec cargo +nightly fmt "${pkg_args[@]}" -- --check
