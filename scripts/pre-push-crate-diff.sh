#!/usr/bin/env bash
set -euo pipefail

# Determine comparison range for this push.
current_branch="$(git branch --show-current)"
upstream_remote="$(git config --get "branch.${current_branch}.remote" 2>/dev/null || true)"
upstream_merge="$(git config --get "branch.${current_branch}.merge" 2>/dev/null || true)"

if [[ -n "$upstream_remote" && -n "$upstream_merge" ]]; then
  upstream_branch="$(printf '%s' "$upstream_merge" | sed 's#^refs/heads/##')"
  range="$upstream_remote/$upstream_branch...HEAD"
elif git rev-parse --verify origin/main >/dev/null 2>&1; then
  range="origin/main...HEAD"
else
  echo "lefthook: no upstream ref found; running fallback smoke gate"
  cargo nextest run -p nebula-core -p nebula-engine -p nebula-execution --profile agent
  cargo check --workspace --all-features --all-targets --quiet
  exit 0
fi

changed_files="$(git diff --name-only "$range" || true)"
changed_crates="$(printf '%s\n' "$changed_files" | sed -n 's#^crates/\([^/]*\)/.*#\1#p' | sort -u)"

if [[ -z "$changed_crates" ]]; then
  echo "lefthook: no crate changes in $range; skipping pre-push crate checks"
  exit 0
fi

pkg_args=()
existing_crates=()
while IFS= read -r crate; do
  [[ -z "$crate" ]] && continue
  # Skip deleted crates: if Cargo.toml no longer exists in the working tree,
  # the package was removed (e.g. crate consolidation per ADR) and there is
  # nothing to test for it.
  if [[ ! -f "crates/$crate/Cargo.toml" ]]; then
    echo "lefthook: skipping deleted crate nebula-$crate"
    continue
  fi
  pkg_args+=("-p" "nebula-$crate")
  existing_crates+=("$crate")
done <<<"$changed_crates"

if [[ ${#pkg_args[@]} -eq 0 ]]; then
  echo "lefthook: all changed crates were deletions; nothing to test"
  exit 0
fi

echo "lefthook: checking changed crates: ${existing_crates[*]}"
cargo nextest run "${pkg_args[@]}" --profile agent
cargo check "${pkg_args[@]}" --all-features --all-targets --quiet

# Keep no-default-features checks for crates that support this gate.
for crate in resilience log expression; do
  if printf '%s\n' "${existing_crates[@]}" | rg -x "$crate" >/dev/null; then
    cargo check -p "nebula-$crate" --no-default-features --quiet
  fi
done

# DATABASE_URL-gated PG storage tests (M2.2 / M3.4 contract). When the
# operator has a Postgres reachable, run the `feature = "postgres"`
# integration tests for nebula-storage to catch concurrency / migration
# regressions before push. When `DATABASE_URL` is unset, emit a single
# WARN line and skip — exit 0 so dev machines without Postgres don't
# fail on this gate.
if printf '%s\n' "${existing_crates[@]}" | rg -x "storage" >/dev/null; then
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
