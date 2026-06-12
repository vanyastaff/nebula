# nebula-storage-loom-probe — Agent orientation
> Agent quick-map for `crates/storage-loom-probe/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Standalone loom model-checker that re-implements `nebula-storage`'s CAS critical sections (credential refresh-claim + execution-lease handoff) against `loom::sync` and proves their single-owner invariants.
**Layer:** Exec — depends only downward (root AGENTS.md -> Layered Dependency Map). Has **zero `nebula-storage` dep on purpose** (see below); not consumed by any production crate.

## Commands
- `cargo check -p nebula-storage-loom-probe` (cheap: whole crate is `#![cfg(loom)]`, so this compiles to nothing without `--cfg loom`)
- Run the probes (loom is dev-only, behind the `loom-test` feature + `--cfg loom`):
  - `RUSTFLAGS="--cfg loom" cargo nextest run -p nebula-storage-loom-probe --features loom-test --profile ci --no-tests=pass`
  - single probe: append `--test refresh_claim_loom` or `--test lease_handoff_loom`
- doctests: none (`[lib] doctest = false`)

## Key files
- `src/lib.rs` — refresh-claim probe: `Repo::try_claim` (`Mutex<HashMap<u32, ClaimRow>>`), mirrors `InMemoryRefreshClaimRepo::try_claim`
- `src/lease_handoff.rs` — execution-lease probe: `LeaseRepo::{acquire,renew,release}_lease`, mirrors `InMemoryExecutionStore` lease ops (fencing-generation, not holder-string, fenced)
- `tests/refresh_claim_loom.rs` / `tests/lease_handoff_loom.rs` — the loom model-check harnesses

## Conventions & never-do
- **Do NOT add a `nebula-storage` (or any non-`loom`) dependency.** `--cfg loom` leaks to every crate in the build; a transitive dep (`concurrent-queue` via `moka`) would break. `loom` is the only allowed runtime dep, kept `optional` behind `loom-test`.
- Probes **mirror** production CAS shapes by hand — they must stay invariant-equivalent (e.g. `generation` == the store's `fencing_generation`); update the mirror when the real adapter's CAS changes, don't diverge silently.
- This crate is probes only — no production storage logic here (that lives in `nebula-storage` / `nebula-storage-port`, ADR-0072). New probes land here under the same sibling-crate / `#![cfg(loom)]` / `loom-test` discipline.
- Loom doesn't model time: TTL/expiry is an explicit `expired: bool` flag, not a deadline.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code (probe bodies are loom test scaffolding, exempt like tests).

## See also
- `README.md` — full design, per-probe table, why-standalone rationale
- `crates/storage/`, `crates/storage-port/` — the production adapter + Core seam these probes mirror (ADR-0072)
- ADR-0041 (`docs/adr/HISTORICAL.md`) — original refresh-claim design
