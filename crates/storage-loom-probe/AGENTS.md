# nebula-storage-loom-probe — Agent orientation
> Local guide for `crates/storage-loom-probe/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Standalone loom model-checker that re-implements `nebula-storage`'s CAS critical sections (credential refresh-claim + execution-lease handoff) against `loom::sync` and proves their single-owner invariants.
**Layer:** Exec — depends only downward (root AGENTS.md -> Layered Dependency Map). Has **zero `nebula-storage` dep on purpose** (see below); not consumed by any production crate.

## Commands

- `cargo check -p nebula-storage-loom-probe` (cheap: whole crate is `#![cfg(loom)]`, so this compiles to nothing without `--cfg loom`)
- Run the probes (loom is an optional dependency behind `loom-test`; both the feature and `--cfg loom` are required):
  - `RUSTFLAGS="--cfg loom" cargo nextest run -p nebula-storage-loom-probe --features loom-test --profile ci`
  - single probe: append `--test refresh_claim_loom` or `--test lease_handoff_loom`
- doctests: none (`[lib] doctest = false`)
- Check that both probe targets actually run. Do not use `--no-tests=pass` as evidence for a model-checking change; a normal cfg-free build exercises no probe code.

## Key files

- `src/lib.rs` — refresh-claim probe: `Repo::try_claim` (`Mutex<HashMap<u32, ClaimRow>>`), mirrors `InMemoryRefreshClaimRepo::try_claim`
- `src/lease_handoff.rs` — execution-lease probe: `LeaseRepo::{acquire,renew,release}_lease`, mirrors `InMemoryExecutionStore` lease ops (fencing-generation, not holder-string, fenced)
- `tests/refresh_claim_loom.rs` / `tests/lease_handoff_loom.rs` — the loom model-check harnesses

## Conventions & never-do

- **Do NOT add a `nebula-storage` (or any non-`loom`) dependency.** `--cfg loom` leaks to every crate in the build; a transitive dep (`concurrent-queue` via `moka`) would break. `loom` is the only allowed runtime dep, kept `optional` behind `loom-test`.
- Probes **mirror** production CAS shapes by hand — they must stay invariant-equivalent (e.g. `generation` == the store's `fencing_generation`); update the mirror when the real adapter's CAS changes, don't diverge silently.
- This crate is probes only — no production storage logic here (that lives in `nebula-storage` / `nebula-storage-port`, ADR-0072). New probes land here under the same sibling-crate / `#![cfg(loom)]` / `loom-test` discipline.
- Loom doesn't model time: TTL/expiry is an explicit `expired: bool` flag, not a deadline.
- Probe bodies are loom test scaffolding — the root no-panic-in-lib rule does not apply here, same as for tests.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Refresh claim CAS | [refresh_claim_loom](tests/refresh_claim_loom.rs); compare the modeled branch conditions and fields with the production refresh-claim adapter. |
| Execution lease CAS | [lease_handoff_loom](tests/lease_handoff_loom.rs); compare generation fencing with the production execution store. Passing a mirror does not prove unmodeled production behavior. |

## See also

- `README.md` — full design, per-probe table, why-standalone rationale
- `crates/storage/`, `crates/storage-port/` — the production adapter + Core seam these probes mirror (ADR-0072)
- ADR-0041 (the ADR history (maintainers' private design vault)) — original refresh-claim design
