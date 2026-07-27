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
- `package.metadata.nebula.ci.test-features` affects tests only; it never changes
  check, documentation, or dependency resolution policy.
- Consumers may name packages in an independent, documented gate policy only
  after plan selection. The current no-default-feature policy names
  `nebula-resilience`, `nebula-log`, `nebula-expression`, `nebula-credential`,
  `nebula-resource`, and `nebula-storage`; they never influence selector
  membership.
- `north-star-gates validate` reads the repository-owned registry, evidence
  schema, canonical multi-run evidence, and checked-in workflow job IDs only.
  It is post-selection gate policy: it never selects packages or changes the
  `ci-plan` result.
- North Star registry v1 rejects `passed`. Evidence schema v1 records a
  bounded, versioned policy/evidence shape, but its CI identity is not a
  trusted attestation and its threshold result is not recomputed from raw
  observations. Registry state remains `red`, `partial`, or `missing` until a
  later schema can represent trustworthy promotion.
- `runtime-repair-red verify` is expected-failure evidence policy, not a test
  runner. It accepts only the raw nextest test-failure exit, exact manifest
  identities, ordinary failures, and exact reason markers from bounded JUnit.
  It never accepts ignored, skipped, retried, timed-out, sentinel, or synthetic
  production evidence.

## Verification

```bash
cargo nextest run -p nebula-xtask
cargo clippy -p nebula-xtask --all-targets -- -D warnings
cargo xtask ci-plan full | jq .
cargo xtask north-star-gates validate
cargo xtask runtime-repair-red validate-manifest
```
