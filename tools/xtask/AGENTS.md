# AGENTS.md — nebula-xtask

Read the repository-root `AGENTS.md` first.

## Purpose

Repository-only automation that derives CI package plans from Cargo metadata.
It is not a product crate and does not participate in the product layer map.

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

## Verification

```bash
cargo nextest run -p nebula-xtask
cargo clippy -p nebula-xtask --all-targets -- -D warnings
cargo xtask ci-plan full | jq .
```
