---
name: nebula-storage-loom-probe
role: Standalone loom-checked concurrency probes for storage critical sections
status: partial
last-reviewed: 2026-05-26
related: [nebula-storage, nebula-credential, nebula-execution]
---

# nebula-storage-loom-probe

## Purpose

`nebula-storage-loom-probe` is a **standalone, loom-checked sibling
crate** that hosts concurrency-model-checker probes for critical
sections in `nebula-storage`. As of today it ships two probes
(verify with `ls crates/storage-loom-probe/src/` and
`ls crates/storage-loom-probe/tests/`):

| Probe | Module | Test file | What it proves |
|---|---|---|---|
| Refresh-claim CAS | `lib.rs` body (`ClaimRow`, `Mutex<HashMap<…>>`) | `tests/refresh_claim_loom.rs` | At-most-one-valid-claim per `credential_id` under the production `InMemoryRefreshClaimRepo::try_claim` shape. |
| Execution-lease handoff | `pub mod lease_handoff` | `tests/lease_handoff_loom.rs` | Single-owner invariant for the execution-lease handoff dance. |

Both probes mirror the production shape locally — the crate
**intentionally has no `nebula-storage` dependency** (see "Why a
standalone crate?" below); shapes are re-implemented inside this
crate against `loom::sync`.

## Why a standalone crate?

Setting `RUSTFLAGS="--cfg loom"` propagates to **every crate in the
build**, including transitive deps like `concurrent-queue` (pulled in
via `moka` → `async-lock` → `event-listener` from `nebula-storage`).
Those crates have their own `cfg(loom)` blocks that import `loom::sync`
from their own dep graph — they never declared loom, so the build
fails when the cfg leaks in transitively.

This probe is a sibling crate that depends **only on `loom`** (and
only when the `loom-test` feature is on), so `--cfg loom` activates
code only where `loom` is in scope. The probe re-creates the CAS
shape used in
`nebula_storage::credential::InMemoryRefreshClaimRepo::try_claim`
(a `Mutex<HashMap<id, ClaimRow>>` + expiry check + write) with
`loom::sync::Mutex` so the model checker can explore lock acquisition
itself.

## Layer

The crate sits in the **Exec** layer per CLAUDE.md § "Layered
Dependency Map". Not consumed by any production crate; runs only under
`RUSTFLAGS="--cfg loom"` in dev / CI.

## Running the probes

The crate exposes the `loom-test` Cargo feature
(`crates/storage-loom-probe/Cargo.toml`); enable it together with the
`--cfg loom` rustc flag.

Both probes (run separately or together):

```bash
RUSTFLAGS="--cfg loom" cargo nextest run \
  -p nebula-storage-loom-probe \
  --features loom-test \
  --profile ci --no-tests=pass
```

Only one probe at a time:

```bash
# Refresh-claim CAS
RUSTFLAGS="--cfg loom" cargo nextest run \
  -p nebula-storage-loom-probe --features loom-test \
  --profile ci --no-tests=pass \
  --test refresh_claim_loom

# Execution-lease handoff
RUSTFLAGS="--cfg loom" cargo nextest run \
  -p nebula-storage-loom-probe --features loom-test \
  --profile ci --no-tests=pass \
  --test lease_handoff_loom
```

The crate root and probe test files are gated by `#![cfg(loom)]`, so a
normal `cargo check -p nebula-storage-loom-probe` (no feature, no cfg)
is cheap and does not require the loom dep at all.

## Out of scope

- Production storage code — see `nebula-storage` (Exec adapter) and
  `nebula-storage-port` (Core seam, ADR-0072).
- Probes for invariants outside the two listed above. Each new probe
  should land here under the same sibling-crate / `cfg(loom)` /
  `loom-test` feature discipline.

## Related

- `crates/storage/` — production adapter whose CAS patterns these
  probes mirror.
- `crates/storage-port/` — Core storage seam (ADR-0072).
- `crates/credential/` — the credential contract whose refresh-claim
  pattern motivates the first probe.
- `crates/execution/` — owns the execution-lease lifecycle whose
  handoff motivates the second probe.
- ADR-0041 (historical, `docs/adr/HISTORICAL.md`) — original
  refresh-claim design.
- ADR-0072 — live `storage-port` / `storage` / `tenancy` redesign.
