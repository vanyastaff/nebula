# nebula-credential cleanup — P6–P11 status (storage / engine / API / consumers)

> **Why this file exists:** [ADR-0032](../../adr/0032-credential-store-canonical-home.md) and the
> [credential architecture cleanup design](../specs/2026-04-20-credential-architecture-cleanup-design.md)
> describe phased work **after** P1–P5. The P1–P5 plan stops at ADR landing; **§12 of the design spec**
> maps **P6–P11**. This document is the **rolled-up execution record** for those phases so agents do
> not re-plan completed migrations.

**Sibling document:** [2026-04-20-credential-cleanup-p1-p5.md](./2026-04-20-credential-cleanup-p1-p5.md) (duplicate collapse through ADRs 0028–0031).

---

## ADR-0032 (P6 dependency-cycle resolution)

| Decision | In-tree |
|----------|---------|
| `CredentialStore` trait + `StoredCredential` / `PutMode` / `StoreError` stay in `nebula-credential` | ✅ `crates/credential/src/store.rs` |
| Concrete stores, layers, `KeyProvider` live in `nebula-storage::credential` | ✅ `crates/storage/src/credential/` |
| **No** `nebula-credential → nebula-storage` edge | ✅ (enforced — credential cannot depend on storage) |
| Cycle-safe shims `store_memory`, `pending_store_memory` remain in credential for internal/tests | ✅ per ADR-0032 §3 |

---

## Spec §12 phase checklist

| Phase | Spec scope | Status | Evidence |
|-------|------------|--------|----------|
| **P6** | Storage `credential` module: memory, layers, key provider | **Landed** | `crates/storage/src/credential/{memory.rs,key_provider.rs,layer/}` |
| **P7** | Pending + backup repos in storage | **Landed** | `pending.rs`, `backup.rs` (feature-gated with storage features) |
| **P8** | Engine `credential/`: resolver, registry, executor, rotation orchestration | **Landed** | `crates/engine/src/credential/` (incl. `rotation/scheduler.rs`); engine `feature = "rotation"` |
| **P9** | Engine token refresh HTTP (`reqwest`) | **Landed** | `crates/engine/src/credential/rotation/token_refresh.rs`; `reqwest` in `crates/engine/Cargo.toml` |
| **P10** | API OAuth HTTP ceremony | **Landed** | `crates/api/src/credential/{oauth_controller.rs,flow.rs,state.rs,mod.rs}` |
| **P11** | Consumers + MATURITY honesty | **Landed** | Imports use `nebula_credential::…` / `nebula_storage::credential::…`; `docs/MATURITY.md` row for `nebula-credential` documents engine-owned runtime |

---

## Follow-ups (non-blocking)

1. **`oauth2-http` + `credentials/oauth2/flow.rs`** — Optional HTTP in the contract crate remains behind `oauth2-http` (default on). README and ADR-0031 describe incremental transport narrowing vs engine/API when ready.
2. **Design spec §2 “final shape”** — Lists `rotation/` in credential as contract-only; **blue_green / grace_period / transaction** modules still live in `nebula-credential::rotation` with types re-exported through `nebula_engine::credential::rotation` for orchestration consumers. A future physical move of those files into `engine/` is optional (same public paths can be preserved via re-exports).
3. **Base-dep diet** — Spec §9 target removes `reqwest`/`url` from credential entirely; today `reqwest` is optional (`oauth2-http`), `url` supports authorization URL construction without HTTP. Further trimming is a follow-on PR, not an open P6–P11 blocker.

---

## Verification

**Last full gate (representative):** `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo nextest run --workspace`; `cargo test --workspace --doc`.

---

## Related

- Design: [2026-04-20-credential-architecture-cleanup-design.md](../specs/2026-04-20-credential-architecture-cleanup-design.md) (§3 file moves, §12 phases)
- ADR: [0028](../../adr/0028-cross-crate-credential-invariants.md), [0029](../../adr/0029-storage-owns-credential-persistence.md), [0030](../../adr/0030-engine-owns-credential-orchestration.md), [0031](../../adr/0031-api-owns-oauth-flow.md), [0032](../../adr/0032-credential-store-canonical-home.md)
