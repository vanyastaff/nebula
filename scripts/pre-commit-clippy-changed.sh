#!/usr/bin/env bash
# Lint selected owners; standalone contract fixtures execute their owner harness.
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/pre-commit-common.sh"
load_pre_commit_plan "$@"
[[ -n "$pre_commit_plan_json" ]] || exit 0

# Capture every extraction before starting checks: process substitutions hide
# producer failures, and a here-string alone turns empty output into one item.
package_lines="$(jq -r '.packages[]' <<< "$pre_commit_plan_json")"
manifest_lines="$(jq -r '.standalone_manifests[]' <<< "$pre_commit_plan_json")"
contract_lines="$(jq -r '
  .fixtures | unique_by([.owner, .test_target])[] | .owner, .test_target
' <<< "$pre_commit_plan_json")"
packages=()
manifests=()
contracts=()
if [[ -n "$package_lines" ]]; then
  mapfile -t packages <<< "$package_lines"
fi
if [[ -n "$manifest_lines" ]]; then
  mapfile -t manifests <<< "$manifest_lines"
fi
if [[ -n "$contract_lines" ]]; then
  mapfile -t contracts <<< "$contract_lines"
fi

package_args=()
for package in "${packages[@]}"; do
  package_args+=(-p "$package")
done
if [[ ${#package_args[@]} -gt 0 ]]; then
  echo "clippy (owners): ${packages[*]}"
  cargo clippy --locked "${package_args[@]}" --all-targets -q -- -D warnings
fi

for manifest in "${manifests[@]}"; do
  echo "clippy (standalone): $manifest"
  cargo clippy --manifest-path "$manifest" --all-targets -q -- -D warnings
done

for ((index = 0; index < ${#contracts[@]}; index += 2)); do
  owner="${contracts[$index]}"
  target="${contracts[$((index + 1))]}"
  echo "clippy and assertions (owner contract): $owner/$target"
  cargo nextest run --locked -p "$owner" --test "$target" -j 1 --no-tests=fail
done
