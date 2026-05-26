---
name: nebula-storage-loom-probe
role: Loom-checked concurrency probe for storage critical sections
status: partial
last-reviewed: 2026-05-26
related: [nebula-storage, nebula-storage-port, nebula-credential]
---

# nebula-storage-loom-probe

## Purpose

`nebula-storage-loom-probe` is a **standalone, loom-checked probe**
that re-creates the CAS shape used by the credential refresh-claim
pattern (ADR-0041) and exercises it under `loom`'s concurrency model
checker. It exists so the refresh-coordinator's atomicity invariant
is *proven*, not just unit-tested.

## Why a standalone crate?

Setting `RUSTFLAGS="--cfg loom"` propagates to **every crate in the
build**, including transitive deps like `concurrent-queue` (pulled in
via `moka` → `async-lock` → `event-listener` from `nebula-storage`).
Those crates have their own `cfg(loom)` blocks that import `loom::sync`
from their own dep graph — they never declared loom, so the build
fails when the cfg leaks in transitively.

This probe is a sibling crate that depends **only on `loom`**, so
`--cfg loom` activates code only where `loom` is in scope. The probe
mirrors `nebula_storage::credential::InMemoryRefreshClaimRepo::try_claim`
shape (`Mutex<HashMap<id, ClaimRow>>` + expiry check + write) with
`loom::sync::Mutex` so the model checker can explore lock acquisition
itself.

## Layer

The crate sits in the **Exec** layer per CLAUDE.md § "Layered Dependency
Map" — it imports the credential-claim shape from `nebula-storage` but
otherwise stays leaf. Not consumed by any production crate; runs only
under `RUSTFLAGS="--cfg loom"` in CI / dev.

## Running the probe

```bash
RUSTFLAGS="--cfg loom" cargo nextest run \
  -p nebula-storage-loom-probe \
  --features loom
```

The probe file itself is gated by `#![cfg(loom)]`, so a normal
`cargo check -p nebula-storage-loom-probe` is cheap and does not
require the loom dep.

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
- ADR-0041 — refresh-coordinator design that drives this probe.
- ADR-0072 — `storage-port` / `storage` / `tenancy` redesign.
