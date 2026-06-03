---
name: nebula-storage-port-adapter
description: Use when changing storage, repository traits, CAS / leases / outbox / journal / idempotency, the spec-16 port, the SQLite/Postgres adapter, or tenant scoping of stores.
---

# nebula-storage-port-adapter

Storage in Nebula is a **three-crate split** under ADR-0072
(`docs/adr/0072-nebula-storage-spec16-port-adapter-tenancy.md`). The legacy
`ExecutionRepo` / `WorkflowRepo` / `Pg*Repo` dual layer and the
never-implemented `repos::{execution,workflow,execution_node,journal}`
placeholders were **deleted** — everything runs on the port. Do not resurrect
them.

## When to use

Adding or modifying a storage repository method; touching the execution CAS /
lease handoff / control-queue outbox / journal / idempotency / refresh-claim
machinery; implementing a backend; or wrapping a store for tenant scoping.

## The three parts

| Crate | Tier | Role | Holds |
|---|---|---|---|
| `nebula-storage-port` (`crates/storage-port/`) | Core | Pure contract — declares *what* storage does, **no backend code** | object-safe `#[async_trait]` traits, port-local DTO rows, plain-data `Scope { workspace_id, org_id }`, `StorageError`, `FencingToken`, the `TransitionBatch` unit-of-work |
| `nebula-storage` (`crates/storage/`) | Exec | The **SOLE** adapter — implements the port | `inmem::*` (tests / single-process / loom), `sqlite::*` (feature `sqlite`), `postgres::*` (feature `postgres`); sqlx, migrations, pool, credential layer; residual `repos::*` (control-queue / idempotency / webhook / identity glue) |
| `nebula-tenancy` (`crates/tenancy/`) | Business | Scope-substituting **decorator** + policy | `ScopeResolver` (`Principal` → `Scope`), one `Scoped*Store` per port trait, `request_scope(&TenantContext)` |

Dependency direction (`crates/storage-port/README.md`): `engine` / `api` /
`core` depend **only on the port**; only composition roots
(`nebula-api` `AppState`, the engine wiring, the knife test) wire the concrete
adapter + tenancy decorator. The `deny.toml [wrappers]` blocks for
`nebula-storage-port` (broad, Core-tier), `nebula-storage` (engine/api/server
composition seam), and `nebula-tenancy` (api/engine + scoped-conformance
dev-dep only) lock the exact consumer sets — a sibling crate cannot quietly
bypass the boundary.

## Procedure: changing the storage contract

1. **Port FIRST.** Edit the trait in `crates/storage-port/src/store/*.rs`
   (`execution.rs`, `workflow.rs`, `control_queue.rs`, `node_result.rs`,
   `journal.rs`, `idempotency.rs`, `checkpoint.rs`, `refresh_claim.rs`,
   `identity.rs`, `webhook.rs`). Add/adjust DTO rows in
   `crates/storage-port/src/dto/` — DTOs depend only on `serde_json::Value`,
   **never** on `ActionResult` or any higher-tier type (Core-tier inversion).
   Keep traits `#[async_trait]` + `dyn`-compatible (consumed as `Arc<dyn …>`).
   `cargo check -p nebula-storage-port`.
2. **Implement in ALL THREE backends.** `crates/storage/src/inmem/`,
   `src/sqlite/` (feature `sqlite`), `src/postgres/` (feature `postgres`).
   Never add backend-specific behavior to the port — the port declares
   behavior; backends realize it identically (SQLite parity = API +
   single-writer correctness, not concurrency/throughput parity, ADR-0072
   decision 7). `cargo check -p nebula-storage --features sqlite,postgres`.
3. **Add a tenancy decorator** if the method is tenant-scoped: a
   `Scoped*Store` in `crates/tenancy/src/decorator/*.rs` that **substitutes**
   the bound `Scope` on the call. Never compare-and-reject (existence oracle);
   let the backend filter `WHERE workspace_id=? AND org_id=?` and surface an
   id↔scope mismatch as `NotFound` / `Ok(None)`.
4. **Run the conformance matrix** — one behavioral suite ×
   {InMemory, SQLite, Postgres} × the scoped decorator
   (`crates/storage/tests/conformance`, scoped variant dev-deps
   `nebula-tenancy`). `cargo nextest run -p nebula-storage`.

## Atomicity & CAS rules (non-negotiable)

- **`ExecutionStore::commit` is the single source of truth** for execution
  state (canon §11.1). It applies CAS on `version` and gates every transition
  with the lease `FencingToken`. If persistence is unavailable it **fails** —
  it never silently mutates in-memory state.
- **`TransitionBatch` is the §12.2 atomic unit-of-work** — state CAS + control
  outbox + journal append committed as **one** logical operation. Its fields
  are private and builder-only (`crates/storage-port/src/batch.rs`), so a
  caller cannot transition without declaring scope + expected version +
  fencing token. Never transition state without enqueueing the outbox, and
  never enqueue without transitioning.
- **Lease fencing closes the zombie-runner hole** (ADR-0072 bug #1).
  `acquire_lease` returns a monotone `FencingToken`; the engine threads it into
  every committed `TransitionBatch`, so a superseded holder is rejected **even
  on a matching CAS version**.
- **Refresh-claim CAS** (ADR-0041, `crates/storage/src/credential/refresh_claim/`):
  `try_claim` must be atomic under contention (exactly one of N concurrent
  acquirers across N replicas wins); `heartbeat` must validate
  `ClaimToken.generation` so a stale holder cannot extend a reclaimed claim
  (reclaim sweep bumps generation).

## Tenant scoping rules

- The tenancy decorator substitutes the `Scope` on **every call before it
  reaches a handler**. The engine/api receive an **already-scoped**
  `Arc<dyn …Store>`; the raw adapter constructor is crate-private to wiring —
  a confused-deputy caller is structurally unable to forge another tenant's
  scope. **Never bypass the decorator.**
- `Scope` is plain data and lives in the **port** (Core tier), so port
  signatures can require it without an upward dep. `nebula-tenancy` owns
  *policy* only — resolving `Scope` from a `Principal` (`BindingScopeResolver`
  is fail-closed: an absent workspace binding is rejected, never widened to
  org-only) and cross-tenant denial. It must **not** own the `Scope` type and
  **not** add a backend/sqlx dependency.
- Cross-tenant access returns `NotFound` (never another tenant's row, never a
  distinct "denied" that leaks existence), proven by the cross-tenant-denial
  conformance suite (spec §6.1). Idempotency keys are tenant-namespaced
  (`{scope}:{key}`) so a replay-oracle cannot probe another tenant's dedup.

## Concurrency proof obligation (loom)

`nebula-storage-loom-probe` (`crates/storage-loom-probe/`) re-implements
storage CAS critical sections against `loom::sync` and proves their
single-owner invariant. **Adding a new CAS critical section ⇒ add a probe
here.** Two probes today: refresh-claim CAS (`src/lib.rs`,
`tests/refresh_claim_loom.rs`) and execution-lease handoff
(`src/lease_handoff.rs`, `tests/lease_handoff_loom.rs`). The crate has **zero
`nebula-storage` dep on purpose** (`--cfg loom` leaks to every crate; a
transitive dep like `concurrent-queue` via `moka` would break) — probes
**mirror** the production shape by hand and must stay invariant-equivalent
(probe `generation` == the store's `fencing_generation`); update the mirror
when the real adapter's CAS changes. Run:

```bash
RUSTFLAGS="--cfg loom" cargo nextest run -p nebula-storage-loom-probe \
  --features loom-test --profile ci --no-tests=pass
```

## Migrations

Two per-backend trees: `crates/storage/migrations/postgres/` and
`crates/storage/migrations/sqlite/` (logically identical tables; dialect types
differ). **No flat top-level tree.** The spec-16 port adapters persist through
`port_*` tables in `0027_port_adapter_schema.sql`, which must stay
**byte-identical** to the embedded `src/{postgres,sqlite}/schema.sql` that
`init_schema` applies for `:memory:` / test pools (regenerate the migration
with `cp` from the embedded schema — see each tree's README). `task db:migrate`
applies pending Postgres migrations (`DATABASE_URL`-gated); `task db:reset`
drops and recreates the DB (destroys local dev data).

## Verification & honesty

- InMemory + SQLite are runtime-verified. **Postgres is
  done-but-pg-unverified** — `DATABASE_URL`-gated and skip-clean in the
  worktree (e.g. `crates/storage/tests/pg_idempotency.rs`). Never claim
  pg-verified without a live DB.
- `engine` still uses a fixed `engine_scope() = Scope::new("nebula", "nebula")`
  placeholder (≈20 call sites), relying on the tenancy decorator to substitute
  the request scope; per-execution engine tenant scoping is a tracked,
  deliberately deferred follow-up (ADR-0072 "Known follow-up"). The api/port/
  tenancy boundary is already per-request and conformance-tested.

## Never do

- Re-add the deleted `ExecutionRepo` / `WorkflowRepo` / `Pg*Repo` surface or
  the `repos::{execution,workflow,execution_node,journal}` placeholders.
- Implement a backend inside `nebula-storage-port`, or put DTO deps on
  higher-tier types.
- Add backend-specific behavior to the port instead of to all three adapters.
- Let a consumer reach the raw adapter and bypass the tenancy decorator.
- Use `unwrap()` / `expect()` / `panic!()` in lib code — typed
  `StorageError` / `thiserror` only.
