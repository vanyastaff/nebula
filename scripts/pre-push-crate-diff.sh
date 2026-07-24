#!/usr/bin/env bash
set -euo pipefail

# Determine the same comparison mode used by pull-request CI. Git resolves the
# configured upstream so local remotes and custom fetch refspecs retain their
# native semantics. Only verified commit IDs cross the selector boundary.
plan_args=(ci-plan full)
comparison_label="full workspace (no upstream)"
if upstream_sha="$(git rev-parse --verify --quiet '@{upstream}^{commit}')"; then
  if ! upstream_label="$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null)" \
    || [[ -z "$upstream_label" ]]; then
    upstream_label="${upstream_sha:0:12}"
  fi
  plan_args=(ci-plan diff --base "$upstream_sha" --head HEAD --comparison merge-base)
  comparison_label="$upstream_label...HEAD"
elif origin_main_sha="$(git rev-parse --verify --quiet 'origin/main^{commit}')"; then
  plan_args=(ci-plan diff --base "$origin_main_sha" --head HEAD --comparison merge-base)
  comparison_label="origin/main...HEAD"
fi

plan_json="$(cargo xtask "${plan_args[@]}")"
if (( ${#plan_json} > 460800 )); then
  echo "lefthook: CI plan exceeds the conservative 450 KiB output limit" >&2
  exit 1
fi

if ! jq -e '
  ((keys | sort) == ["count", "include", "reason", "schema_version", "scope"]) and
  (.schema_version == 1) and
  (.scope == "full" or .scope == "diff") and
  ((.reason | type) == "string") and
  ((.count | type) == "number") and
  (.count >= 0 and .count <= 256 and .count == (.count | floor)) and
  ((.include | type) == "array") and
  (.count == (.include | length)) and
  ([.include[].package] == ([.include[].package] | sort | unique)) and
  all(.include[];
    ((keys | sort) == ["package", "test_features"]) and
    ((.package | type) == "string") and
    (.package | length) > 0 and
    ((.test_features | type) == "array") and
    (.test_features == (.test_features | sort | unique)) and
    all(.test_features[]; type == "string")
  )
' >/dev/null <<< "$plan_json"; then
  echo "lefthook: nebula-xtask emitted an invalid CI plan" >&2
  exit 1
fi

selected_count="$(jq -r '.count' <<< "$plan_json")"
if [[ "$selected_count" == "0" ]]; then
  echo "lefthook: no package checks required for $comparison_label"
  exit 0
fi

mapfile -t packages < <(jq -r '.include[].package' <<< "$plan_json")
echo "lefthook: checking ${#packages[@]} package(s) for $comparison_label: ${packages[*]}"

# Test one package at a time because test-only feature bundles belong to that
# package's metadata and must never leak into cargo check or rustdoc.
for index in "${!packages[@]}"; do
  package="${packages[$index]}"
  mapfile -t test_features < <(jq -r ".include[$index].test_features[]" <<< "$plan_json")
  test_command=(cargo nextest run -p "$package")
  if (( ${#test_features[@]} > 0 )); then
    saved_ifs="$IFS"
    IFS=,
    joined_features="${test_features[*]}"
    IFS="$saved_ifs"
    test_command+=(--features "$joined_features")
  fi
  test_command+=(--profile agent --no-tests=pass)
  "${test_command[@]}"
done

package_args=()
for package in "${packages[@]}"; do
  package_args+=(-p "$package")
done

# Keep the existing all-target/all-feature static gate for every selected
# package. Test-only feature metadata is intentionally not consulted here.
cargo check "${package_args[@]}" --all-features --all-targets --quiet

echo "lefthook: rustdoc -D warnings for selected packages"
RUSTDOCFLAGS="-D warnings" cargo doc "${package_args[@]}" --no-deps --quiet

package_selected() {
  local wanted="$1"
  local selected
  for selected in "${packages[@]}"; do
    if [[ "$selected" == "$wanted" ]]; then
      return 0
    fi
  done
  return 1
}

# These six names are an independent no-default-feature gate policy, not a
# package-selection list. The metadata plan decides which packages are selected;
# this policy only adds checks for selected packages that promise a minimal
# feature surface.
for package in \
  nebula-resilience \
  nebula-log \
  nebula-expression \
  nebula-credential \
  nebula-resource \
  nebula-storage; do
  if package_selected "$package"; then
    cargo check -p "$package" --no-default-features --quiet
  fi
done

# DATABASE_URL-gated Postgres storage tests preserve the local parity contract.
if package_selected nebula-storage; then
  if [[ -n "${DATABASE_URL:-}" ]]; then
    echo "lefthook: DATABASE_URL set — running PG-gated storage tests"
    cargo nextest run \
      -p nebula-storage \
      --features postgres \
      --test execution_lease_pg_integration \
      --test pg_idempotency \
      --test refresh_claim_pg_integration \
      --profile agent
  else
    echo "lefthook: WARN — DATABASE_URL unset; skipping PG-gated storage tests (pg_idempotency, pg_execution_lease, refresh_claim_pg)"
  fi
fi
