# nebula-storage-port — Agent orientation
> Agent quick-map for `crates/storage-port/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Pure storage contract — object-safe `#[async_trait]` repository traits, port-local DTO rows, plain-data `Scope`, `StorageError`, and the `TransitionBatch` atomic unit-of-work. No backend code.
**Layer:** Core — depends only downward (root AGENTS.md -> Layered Dependency Map). Deps: `nebula-core`, async-trait, serde, chrono, uuid. **No sqlx.**

## Commands
- `cargo check -p nebula-storage-port`
- `cargo nextest run -p nebula-storage-port`  ·  doctests: none (`doctest = false` in Cargo.toml `[lib]`)

## Key files
- `src/lib.rs` — crate root; re-exports `Scope`, `StorageError`, `FencingToken`, `TransitionBatch{,Builder,Outcome}`
- `src/batch.rs` — `TransitionBatch`: private fields, builder-only construction; `commit` writes state+outbox+journal in one CAS+fencing-gated transaction
- `src/store/mod.rs` — ISP-segregated role traits (`ExecutionStore`, `WorkflowStore`, `ControlQueue`, `NodeResultStore`, `ExecutionJournalReader`, identity/idempotency/refresh-claim stores)
- `src/dto/` — port-local row DTOs (execution/workflow/control/journal/node_result/identity/webhook/idempotency), `serde_json::Value`-only
- `src/scope.rs` — plain-data `Scope { workspace_id, org_id }`; `src/ids.rs` — re-exported core ULIDs + lease `FencingToken`

## Conventions & never-do
- This crate declares *what* storage does; **never implement a backend here** (adapters live in `nebula-storage`; scope enforcement in `nebula-tenancy`).
- DTOs depend only on `serde_json::Value` — never on `ActionResult` or any higher-tier type (avoids Core-tier dependency inversion).
- `Scope` is a value type with **no policy**; resolving it from a principal and cross-tenant denial belong to `nebula-tenancy`, not here.
- Every repository trait stays `#[async_trait]` + `dyn`-compatible (consumed as `Arc<dyn …>`); keep `TransitionBatch` fields private and builder-only so a transition can't skip scope/CAS/fencing.
- Library code uses typed `thiserror`/`StorageError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · `docs/adr/0072-nebula-storage-spec16-port-adapter-tenancy.md` (port/adapter/tenancy contract) · ADR-0041 (RefreshClaimStore shape)
