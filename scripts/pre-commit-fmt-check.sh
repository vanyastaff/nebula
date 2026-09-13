#!/usr/bin/env bash
# Format selected owners and every source in their selected standalone fixtures.
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/pre-commit-common.sh"
load_pre_commit_plan "$@"
[[ -n "$pre_commit_plan_json" ]] || exit 0

# Validate extraction success before starting any checks, preserving empty arrays.
package_lines="$(jq -r '.packages[]' <<< "$pre_commit_plan_json")"
# A complete fixture includes compile-fail probes, even when only a positive
# source changed. cargo fmt needs no unpublished dependency resolution.
manifest_lines="$(jq -r '
  [.standalone_manifests[], .fixtures[].manifest_path] | sort | unique | .[]
' <<< "$pre_commit_plan_json")"
packages=()
manifests=()
if [[ -n "$package_lines" ]]; then
  mapfile -t packages <<< "$package_lines"
fi
if [[ -n "$manifest_lines" ]]; then
  mapfile -t manifests <<< "$manifest_lines"
fi

package_args=()
for package in "${packages[@]}"; do
  package_args+=(-p "$package")
done
if [[ ${#package_args[@]} -gt 0 ]]; then
  echo "fmt-check (owners): ${packages[*]}"
  cargo fmt "${package_args[@]}" -- --check
fi

for manifest in "${manifests[@]}"; do
  echo "fmt-check (standalone): $manifest"
  cargo fmt --manifest-path "$manifest" -- --check
done
