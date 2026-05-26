---
name: nebula-storage-loom-probe
role: Standalone loom-checked concurrency probe (CAS atomicity)
status: partial
last-reviewed: 2026-05-26
related: [nebula-storage, nebula-credential]
---

# nebula-storage-loom-probe

## Purpose

`nebula-storage-loom-probe` is a **standalone, loom-checked probe**
that re-creates the CAS shape used by the credential refresh-claim
pattern and exercises it under `loom`'s concurrency model checker. It
exists so the refresh-coordinator's atomicity invariant is *proven*,
not just unit-tested.

The CAS shape is implemented locally inside this crate — the design
notes for the underlying refresh-claim pattern lived in ADR-0041
(now historical, see `docs/adr/HISTORICAL.md` row for `0041`); the
production implementation lives in `nebula-storage`. **This probe does
not depend on `nebula-storage`** (see "Why a standalone crate?" below);
the manifest carries only an optional `loom` dependency.

## Why a standalone crate?

Setting `RUSTFLAGS="--cfg loom"` propagates to **every crate in the
build**, including transitive deps like `concurrent-queue` (pulled in
via `moka` → `async-lock` → `event-listener` from `nebula-storage`).
Those crates have their own `cfg(loom)` blocks that import `loom::sync`
from their own dep graph — they never declared loom, so the build
fails when the cfg leaks in transitively.

This probe is a sibling crate that depends **only on `loom`** (and
only when the `loom-test` feature is on), so `--cfg loom` activates
code only where `loom` is in scope. The probe mirrors the
`Mutex<HashMap<id, ClaimRow>>` + expiry check + write shape used by
the production refresh-claim repo with `loom::sync::Mutex` so the
model checker can explore lock acquisition itself.

## Layer

The crate sits in the **Exec** layer per CLAUDE.md § "Layered
Dependency Map". Not consumed by any production crate; runs only under
`RUSTFLAGS="--cfg loom"` in dev / CI.

## Running the probe

The crate exposes the `loom-test` Cargo feature
(`crates/storage-loom-probe/Cargo.toml`); enable it together with the
`--cfg loom` rustc flag:

```bash
RUSTFLAGS="--cfg loom" cargo nextest run \
  -p nebula-storage-loom-probe \
  --features loom-test \
  --profile ci --no-tests=pass
```

The probe file itself is gated by `#![cfg(loom)]`, so a normal
`cargo check -p nebula-storage-loom-probe` (no feature, no cfg) is
cheap and does not require the loom dep at all.

## Out of scope

- Production storage code — see `nebula-storage` (Exec adapter) and
  `nebula-storage-port` (Core seam, ADR-0072).
- Other concurrency invariants — this probe is scoped to the
  refresh-claim CAS shape only. Each probe added in the future should
  follow the same sibling-crate / `cfg(loom)` discipline.

## Related

- `crates/storage/` — the production adapter whose CAS pattern this
  probe mirrors.
- `crates/storage-port/` — Core storage seam (ADR-0072).
- ADR-0041 (historical, `docs/adr/HISTORICAL.md`) — the refresh-claim
  design that motivates this probe.
- ADR-0072 — live `storage-port` / `storage` / `tenancy` redesign.
