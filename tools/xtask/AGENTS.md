# AGENTS.md — nebula-xtask

Read the repository-root `AGENTS.md` first.

## Purpose

Repository-only automation that derives CI package plans from Cargo metadata
and validates versioned post-selection quality-gate policy. It is not a
product crate and does not participate in the product layer map.

## Invariants

- No dependency may be a `nebula-*` product package.
- A successful `ci-plan` writes one compact, deterministic, versioned JSON
  plan. `--help` and `--version` are successful human-readable stdout; invalid
  CLI usage uses Clap's stderr and exit code. Planner failures emit no partial
  stdout.
- Workspace members, package ownership, dependency edges, and declared features
  come from Cargo metadata. Never add a hardcoded package list or infer a Cargo
  package name from a directory name.
- Diff uncertainty widens to the full workspace. Invalid nonempty Git revisions
  remain hard errors so configuration failures are visible.
- `pre-commit-plan` is a separate versioned contract, not a new `ci-plan` scope.
  It reuses deepest Cargo ownership without reverse-dependent expansion.
  Standalone fixture declarations bind only to an existing integration-test
  target on their actual owner; optional owner assertions cannot redirect it.
  Target existence is structural routing, not proof of harness coverage.
  Preserve complete-fixture formatting, positive strict clippy and execution,
  negative diagnostic assertions, and ordinary standalone checks.
- `package.metadata.nebula.ci.test-features` affects tests only; it never changes
  check, documentation, or dependency resolution policy.
- `ci-plan semver` is a separate schema-v1 contract. It reuses diff ownership
  and reverse closure, then selects publishable library targets and requires
  every selected Cargo package name to exist in immutable baseline metadata.
  New or renamed selected packages are hard errors, never skipped entries.
  Sorted names are distributed round-robin across at most three deterministic,
  nonempty shard entries; empty selections emit no shards, and nonempty shard
  count is `min(3, package_count)`. Pull-request CI plans from the checked-out
  synthetic merge commit at `github.sha` against the exact base SHA, so
  base-only changes are not interpreted as pull-request removals.
- Consumers may name packages in an independent, documented gate policy only
  after plan selection. The current no-default-feature policy names
  `nebula-resilience`, `nebula-log`, `nebula-expression`, `nebula-credential`,
  `nebula-resource`, and `nebula-storage`; they never influence selector
  membership.
- `north-star-gates validate` reads the repository-owned registry, evidence
  schema, canonical multi-run evidence, and checked-in workflow job IDs only.
  It is post-selection gate policy: it never selects packages or changes the
  `ci-plan` result.
- North Star registry v1 rejects `passed`; its checked-in state is only the
  baseline. The runtime-authority verifier accepts a complete immutable
  artifact inventory only when runner provenance and every semantic threshold
  match, then emits the deterministic effective `partial` state for each
  covered gate. A failed verification emits no effective-state result.
- `runtime-repair-red verify` is expected-failure evidence policy, not a test
  runner. It accepts only the raw nextest test-failure exit, exact manifest
  identities, ordinary failures, and exact reason markers from bounded JUnit.
  It never accepts ignored, skipped, retried, timed-out, sentinel, or synthetic
  production evidence.

## Verification

```bash
cargo nextest run -p nebula-xtask
cargo nextest run -p nebula-xtask --test pre_commit --test pre_commit_plan -j 1
cargo clippy -p nebula-xtask --all-targets -- -D warnings
cargo xtask ci-plan full | jq .
cargo xtask north-star-gates validate
cargo xtask runtime-repair-red validate-manifest
```
