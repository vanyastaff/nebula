# nebula-storage — spec-16 adoption + port/adapter/tenancy redesign

**Status:** Drafted 2026-05-15. Supersedes the "Sprint E — adopt spec-16 row
model" deferral. Ratified by ADR-0068 (companion, this wave).

**Authority:** brainstorming-led adversarial panel (Rust 1.95 language
engineer, distributed-systems/durability, DDD/boundaries, migrations/sqlx,
security/secrets, delivery-risk pragmatist). The user locked scope
("Полный spec-16 L2" + "Port + adapter + tenancy крейт"), delegated final
panel convergence and the spec-review gate to the implementer, and asked for
an excellent finished crate. This document is the contract the implementation
plan executes against.

---

## 1. Context & Problem

`nebula-storage` carries an **unfinished migration between two incompatible
architectures**:

| Axis | Layer 1 (live, production) | Layer 2 `repos::*` (spec-16, mostly planned) |
|---|---|---|
| IDs | typed `ExecutionId`/`WorkflowId` | raw `&[u8]` (caller-encoded) |
| State | opaque `serde_json::Value` (correct per §11.1) | structured rows + mandatory `workspace_id`/`org_id` |
| Async | `#[async_trait]` | RPITIT `impl Future` |
| Errors | per-repo `ExecutionRepoError`/`WorkflowRepoError` | unified `StorageError` |
| Maturity | wired in engine/api, knife scenario | 3 of ~14 traits implemented; 11 dead placeholders |

Secondary rot: two migration trees (flat `0000…` live; `migrations/{postgres,
sqlite}/*` dead, no consumer); oversized files (`execution_repo.rs` 1872,
`backend/pg_execution.rs` 1202, `pg/control_queue.rs` 1181,
`credential/provider_cache.rs` 1237 — trait+impl+tests fused); `pool.rs`
unused; `ControlQueueEntry.execution_id` encoded as "UTF-8 bytes of the ULID
string" by cross-crate convention (discipline, not types); **canon §12.3
dishonesty** — canon and the README claim "SQLite is the default local path /
one local path" but Layer 1 has no SQLite backend (only InMemory + Postgres);
the actual local path is `InMemoryExecutionRepo`.

**Hard technical collision the chosen scope forces:** the planned spec-16
`repos::ExecutionRepo` is declared with RPITIT (`fn …(&self) -> impl Future +
Send`). RPITIT traits are **not dyn-compatible**, and the entire engine/api
consumes storage as `Arc<dyn …>`. "Adopt spec-16 as written" is therefore
physically impossible; choosing spec-16 *mandates* re-shaping the trait family
into an object-safe form. This is an engineering consequence of the locked
scope, not a scope reduction.

## 2. Decision Summary (panel convergence v3)

Adopt the spec-16 row model as the single storage architecture, decomposed
into **port + adapter + tenancy** crates, with the following non-negotiable
corrections folded in:

1. **Object-safe trait family.** Redesigned, dyn-consumed ports use
   `#[async_trait]` — a deliberate, documented choice: every port call bottoms
   out in network/disk I/O, so the per-call boxed-future allocation is noise;
   `trait_variant`/`dynosaur` would add machinery for zero gain on an
   I/O-bound port. Native `async fn` is reserved for non-dyn adapter-internal
   helpers. `RefreshClaimStore` keeps its current loom-verified shape — no
   idiom churn on a working, proven component.
2. **Crate topology** (`nebula-storage-port` Core / `nebula-storage` Exec /
   `nebula-tenancy` Business) — see §3.
3. **Atomic unit-of-work as a value object** (`TransitionBatch`) — see §4.
4. **Lease fencing gates CAS** — see §4.
5. **Multi-tenancy is a security boundary** with an explicit threat model and
   cross-tenant-denial conformance — see §6.
6. **Port depends only on `nebula-core` + serde** — port-local DTOs, no
   `ActionResult` dependency — see §4/§8.
7. **SQLite parity = API + single-writer correctness, NOT concurrency /
   throughput parity** — see §5.
8. **One behavioral conformance suite** × {InMemory, SQLite, Postgres} ×
   scoped decorator; loom/knife/lease tests ported (not deleted) with a
   written invariant-equivalence note — see §9.

## 3. Target Crate Topology

| Crate | Tier (AGENTS.md) | Content |
|---|---|---|
| **`nebula-storage-port`** (new) | Core | Pure port: ISP-segregated trait family, port-local DTO rows (spec-16 shape), `StorageError`, schema-version constants, `TransitionBatch`, **and the plain-data `Scope { workspace_id, org_id }` value type** (reusing `nebula-core` ID newtypes). Depends only on `nebula-core` + `serde`/`serde_json`/`thiserror`/`async-trait`. **No sqlx.** Object-safe. |
| **`nebula-storage`** (existing) | Exec | Adapters: `InMemory` + **`SQLite`** + `Postgres` impls of the whole port, sqlx, migrations, pool. Depends on `nebula-storage-port`. Credential layer impls stay here. |
| **`nebula-tenancy`** (new) | Business | Multi-tenancy **policy** (not the `Scope` type): `ScopeResolver` (`Principal` → `Scope`, generalised from `credential/layer/scope.rs`) and scoping decorators that wrap the port and enforce isolation. Depends on `nebula-storage-port`. |
| `nebula-storage-loom-probe` | Exec | Stays; re-pointed at the new `InMemory` impls and the new atomic boundary. |

**Dependency direction:** `engine`/`core`/`api` depend **only on
`nebula-storage-port`** (downward, acyclic — engine becomes testable without
sqlx). Only composition roots (api `AppState`, the future `apps/server`, the
knife test) depend on `nebula-storage` (adapter) + `nebula-tenancy`.

**Tension resolution (spec-16 tenant-columns vs separate tenancy crate):**
rows carry `workspace_id`/`org_id` (data model — required for row-level
isolation and scoped queries) and the port defines the plain-data `Scope`
value type so signatures can require it without an upward dependency;
`nebula-tenancy` owns the *policy* (resolve `Scope` from `Principal`, inject,
enforce, deny cross-scope) — never the `Scope` type itself. Data + the scope
type are port/Core-level; policy is the Business-tier cross-cutting security
boundary. This avoids the same dependency inversion §4 forbids for
`ActionResult`.

**`Cargo.toml` / `deny.toml` / AGENTS.md deltas:**
- Add `crates/storage-port`, `crates/tenancy` to workspace `members`.
- `deny.toml [wrappers]`: add a `nebula-storage-port` entry (broad: it is a
  Core-tier contract — engine/api/core/tenancy/storage/storage-loom-probe may
  depend on it). Add `nebula-tenancy` entry (Business; api + composition-root
  dev-deps). Keep `nebula-storage` wrappers as-is (engine/api/credential-vault
  dev-dep) — engine's runtime dep on the *concrete* adapter narrows over time
  but the composition seam stays.
- AGENTS.md Layered Dependency Map + Workspace Layout updated; `+macros`-style
  note not needed (no companion crates).

## 4. Port Trait Family (object-safe, ISP-segregated)

**Design rule:** one atomic aggregate trait owns the §12.2 unit (state
transition + outbox + journal in one logical op); all read-only and
non-atomic concerns are segregated into small role traits so no file/impl
becomes a god-object and consumers depend only on what they use.

### 4.1 The atomic aggregate

```text
trait ExecutionStore (object-safe, #[async_trait]):
  async fn create(scope, ExecutionId, WorkflowId, initial_state) -> Result<(), StorageError>
  async fn get(scope, ExecutionId) -> Result<Option<ExecutionRecord>, StorageError>
  async fn commit(batch: TransitionBatch) -> Result<TransitionOutcome, StorageError>
  async fn acquire_lease(scope, ExecutionId, holder, ttl) -> Result<Option<FencingToken>, StorageError>
  async fn renew_lease(scope, ExecutionId, token: &FencingToken, ttl) -> Result<bool, StorageError>
  async fn release_lease(scope, ExecutionId, token: &FencingToken) -> Result<bool, StorageError>
  async fn list_running(scope) -> Result<Vec<ExecutionId>, StorageError>
  async fn list_running_for_workflow(scope, WorkflowId) -> Result<Vec<ExecutionId>, StorageError>
  async fn count(scope, Option<WorkflowId>) -> Result<u64, StorageError>
```

`TransitionBatch` is a **value object the store consumes** — built via a
typed builder, carrying everything that must commit atomically:

```text
struct TransitionBatch {
  scope: Scope,
  execution_id: ExecutionId,
  expected_version: u64,
  fencing: FencingToken,          // lease fencing — see below
  new_state: serde_json::Value,   // opaque per §11.1
  outbox: Vec<ControlMsg>,        // execution_control_queue rows (§12.2)
  journal: Vec<JournalEntry>,     // execution_journal append (§11.5)
}
```

`commit` applies, in **one DB transaction** (Postgres/SQLite) or **one
mutex-guarded mutation** (InMemory):
- CAS on `version == expected_version` **AND** the fencing token is the
  current lease holder's (a superseded/expired holder is rejected even if the
  version matches — closes the zombie-runner hole);
- write `new_state` + bump version;
- append `outbox` rows and `journal` rows in the same transaction.

`TransitionOutcome ∈ { Applied { new_version }, VersionConflict { actual },
FencedOut }`. Empty `outbox`/`journal` are valid; the point is structural —
there is exactly **one** call site and **one** transaction for the triple, so
the canon §14 "two truths" split cannot occur by construction.

### 4.2 Segregated role traits (all object-safe, `#[async_trait]`)

- `ExecutionJournalReader` — `get_journal`, `list_after`.
- `NodeResultStore` (ADR-0009) — `save_node_output`/`load_*`,
  `save_node_result`/`load_node_result`/`load_all_results`,
  `set_workflow_input`/`get_workflow_input`. Records are **port-local DTOs**
  (`NodeResultRecord { kind_tag, json, schema_version }`) — the port does
  **not** depend on `ActionResult` (prevents Core-tier dependency inversion);
  unknown `schema_version` → `StorageError::UnknownSchemaVersion`.
- `CheckpointStore` (§11.5) — `save_stateful_checkpoint` / `load_*`;
  best-effort semantics documented on the trait.
- `IdempotencyGuard` (§11.3) — check-and-mark; key shape **unchanged**
  `{execution_id}:{node_id}:{attempt}` (ADR-0042 `attempts.len()+1`
  derivation preserved); tenant-namespaced (see §6).
- `WorkflowStore` + `WorkflowVersionStore` — spec-16 workflow/version split.
- `ControlQueue` — `enqueue`/`claim_pending`/`mark_completed`/`mark_failed`/
  `reclaim_stuck`; **typed IDs** (kill the "UTF-8 of ULID string" hack — store
  ULID as `BYTEA(16)`/`BLOB(16)`, decode to the typed ID at the port edge;
  structural fix, not a documented convention). Scoped (see §6).
- `IdempotencyStore` (ADR-0048 hybrid backend) — unchanged contract,
  tenant-namespaced `cache_key`.
- `WebhookActivationStore` (ADR-0049) — unchanged contract.
- `RefreshClaimStore` (ADR-0041) — **shape unchanged** (loom-verified);
  re-homed into the port crate as-is.
- spec-16 identity zoo: `UserStore`, `OrgStore`, `WorkspaceStore`,
  `MembershipStore`, `ResourceStore`, `TriggerStore`, `QuotaStore`,
  `AuditStore`, `BlobStore` — full traits + DTO rows.

`StorageError` (`#[non_exhaustive]`, `thiserror`): `NotFound`, `Conflict`,
`Duplicate`, `LeaseUnavailable`, `FencedOut`, `Timeout`, `UnknownSchemaVersion`,
`Serialization`, `Connection`, `Configuration`, `ScopeViolation`, `Internal`.
The per-repo `ExecutionRepoError`/`WorkflowRepoError` enums are **deleted**
(callers migrate to `StorageError` — breaking, allowed).

## 5. Concurrency / Isolation Contract

The port guarantees an **abstract concurrency contract**, not backend-uniform
behaviour:

- **Ordering primitive:** CAS on `version` is the only cross-process ordering
  guarantee for state. No backend offers stronger.
- **Lease fencing:** `FencingToken` is monotonic per execution; `commit` and
  `renew_lease` reject a non-current token. A reclaim/takeover bumps the
  token generation.
- **Outbox:** at-least-once; consumers must be idempotent (canon §11.3 — no
  exactly-once pretence).
- **Per-backend reality (documented, not hidden):**
  - **Postgres** — production: `commit` uses a real transaction; control-queue
    claim uses `FOR UPDATE SKIP LOCKED` (multi-consumer, ADR-0008 §1).
  - **SQLite** — dev/edge: `commit` uses `BEGIN IMMEDIATE` (single writer);
    control-queue claim is a single-consumer status flip (no `SKIP LOCKED`
    equivalent). **SQLite parity = identical port API + single-writer
    correctness, explicitly NOT concurrent/throughput parity** — this matches
    canon §12.3 ("SQLite for dev/edge; Postgres required for high-throughput").
    Claiming concurrent parity would be a *new* dishonesty replacing the §12.3
    one; the README/MATURITY wording states the boundary precisely.
  - **InMemory** — tests/local: one `parking_lot::Mutex`-guarded mutation per
    `commit`; behaviourally models the single-writer contract; loom probe
    covers the CAS+fencing+outbox boundary.

The §12.3 honesty fix: after this work, "SQLite is the default local path" is
**true for the whole execution/workflow/control/journal/idempotency core**,
not just refresh-claim.

## 6. Multi-tenancy as a Security Boundary

`nebula-tenancy` is a **security boundary**, not a data-shaping convenience.
Threat model and enforcement rules are normative.

### 6.1 Abuse cases (adversarial review — mandatory)

1. **Confused deputy / cross-tenant row access.** Caller supplies a `Scope`
   for workspace A with an `ExecutionId` belonging to B.
   **Mitigation:** every scoped read/transition is
   `WHERE id = ? AND workspace_id = ? AND org_id = ?`. An id↔scope mismatch
   returns `NotFound` (never the row, never `ScopeViolation` leaking
   existence). `Scope` is **non-optional** in the port signature for
   tenant-scoped entities — it cannot be forgotten.
2. **Idempotency replay-oracle.** Tenant A probes/poisons tenant B's dedup
   entry. **Mitigation:** `IdempotencyStore.cache_key` and
   `IdempotencyGuard` keys are tenant-namespaced (`{scope}:{key}`).
3. **Control-queue confused deputy.** Low-priv tenant enqueues a
   Cancel/Terminate for another tenant's execution. **Mitigation:**
   `ControlQueue.enqueue` is scoped; the engine consumer re-verifies scope on
   `claim_pending` against the execution row before dispatch.
4. **Credential scope-layer regression.** Moving scope enforcement out of
   `credential/layer/scope.rs` into `nebula-tenancy` must not drop ADR-0029
   fail-closed audit (audit-write failure → operation fails) or
   zeroize-on-drop, and must preserve pending-store single-use + TTL ≤ 10 min
   + session binding (ADR-0029 §4). Conformance covers cross-tenant pending
   replay denial.

### 6.2 Enforcement shape

Scoping is a **decorator** (in `nebula-tenancy`) wrapping the adapter over the
port-defined `Scope` type, applied at the composition root. `nebula-tenancy`
supplies the `ScopeResolver` (`Principal` → `Scope`) and the enforcing
decorator; it does not own `Scope`. Engine receives only an already-scoped
`Arc<dyn ExecutionStore>` — it is structurally unable to obtain the raw
adapter (the adapter constructor is crate-private to wiring; the decorator is
the only public handle). InMemory impls enforce the same `scope` predicate as
SQL backends so the conformance suite proves cross-tenant denial uniformly.

## 7. Data Model & Migrations

- **Canonical schema:** the per-backend structured trees
  `migrations/{postgres,sqlite}/*` (0001–0026: spec-16 + multi-tenant +
  `idempotency_dedup` + `refresh_claims` + `sentinel` + w3c-trace) become the
  single source. The flat legacy `migrations/0000…` tree is **deleted** after
  porting any drift into the structured tree.
- **Runner:** `sqlx::migrate!()` per backend directory; the adapter selects
  the directory by backend feature.
- **Cutover assumption (explicit):** there is **no deployed production
  database** (pre-1.0; release workflow intentionally absent). The cutover
  drops the old `_sqlx_migrations` lineage; **it destroys existing local dev
  DB state — `task db:reset` is required**. This is locally reversible and is
  recorded as an assumption, not a silent breaking change.
- Dialect divergence is owned per-adapter (JSONB/JSON, BYTEA/BLOB,
  TIMESTAMPTZ vs INTEGER/TEXT, `ON CONFLICT`, no `gen_random_uuid()` in
  SQLite, lease-TTL interval arithmetic). No shared SQL string is assumed
  portable.

## 8. Credential Subsystem Disposition

- `CredentialStore` **stays owned by `nebula-credential`** (shared infra —
  already correct). The port crate does not redefine it.
- `nebula-storage` keeps the impls: `InMemoryStore`, and the composable layer
  stack (`EncryptionLayer`, `CacheLayer`, `AuditLayer`). The `ScopeLayer`
  generalises into `nebula-tenancy`'s `ScopeResolver`; the credential-specific
  wiring re-composes on top with **fail-closed audit and zeroize preserved**
  (regression-tested).
- `ProviderCacheLayer` (ADR-0051 Phase A) stays in `nebula-storage`; the
  1237-line file is split by concern (cache vs config vs resolution).
- `RefreshClaimStore` trait moves to the port; SQLite/Postgres/InMemory impls
  stay in the adapter; loom probe re-pointed. Shape unchanged.

## 9. Conformance, loom, knife, lease

- **Behavioral conformance suite** defined in **P1**:
  `crates/storage/tests/conformance/` — one rstest matrix asserting the §5
  contract (CAS, fencing rejects zombie holder, atomic triple, idempotency
  key shape, cross-tenant `NotFound`) across **{InMemory, SQLite `:memory:`,
  Postgres `DATABASE_URL`-gated}** and across **{raw adapter, scoped
  decorator}**. This is the green target every later phase builds toward.
- **loom probe** (`storage-loom-probe`): extended to cover the new
  CAS+fencing+outbox boundary; existing `lease_handoff`/`refresh_claim`
  probes **ported, not deleted**, with a written invariant-equivalence note in
  the probe crate doc.
- **knife scenario** (`crates/api/tests/knife.rs`) and **lease integration**
  (`crates/engine/tests/lease_takeover.rs`,
  `crates/storage/tests/execution_lease_pg_integration.rs`): ported to the
  new port/scoped seam; the canon §13 end-to-end guarantee is preserved, with
  an equivalence note. No safety test is removed without a proven equivalent
  (operational-honesty rule).

## 10. Quality Bar (Definition of Done)

- No `unwrap`/`expect`/`panic!` in library code (tests/`const`/binaries
  exempt per `clippy.toml`); all production paths return typed `StorageError`.
- Oversized files split by responsibility (trait→port; InMemory impl→own
  module; backend impls split by concern: cas/lease/journal/node-output/
  idempotency; tests→`tests/`). Target: no `src/*.rs` over ~600 lines.
- Every new state/error/hot path ships a typed error variant + a `tracing`
  span + an invariant check (observability is DoD, not follow-up).
- No plan IDs / "Phase X" / task IDs in committed code or comments — comments
  read correctly after this spec is deleted.
- `cargo-deny` layer wrappers updated and green; `task clippy` (`-D
  warnings`) green; `cargo nextest run -p <crate>` green per touched crate.
- **Windows-worktree caveat:** `task dev:check`/`cargo fmt --all` can fail
  with os error 206 in deep Claude worktree paths — verification is done
  **per-crate** (`cargo fmt -p`, `cargo clippy -p`, `cargo nextest run -p`);
  the workspace gate is not reported green from this worktree.

## 11. Phasing (re-sequenced: canon-critical depth before breadth)

- **P1 — Port crate.** `nebula-storage-port`: trait family, `TransitionBatch`,
  `FencingToken`, port-local DTOs, `StorageError`; conformance-suite skeleton
  (compiles, asserts the contract against a stub). Workspace/deny/AGENTS
  wiring for the new crate.
- **P2 — Adapter parity.** `nebula-storage` implements the core
  (execution/control/journal/idempotency/webhook/refresh-claim) for
  **InMemory → SQLite → Postgres**; conformance green for all three (raw).
- **P3 — Tenancy.** `nebula-tenancy`: `Scope`/`ScopeResolver`/decorators;
  threat-model enforcement; conformance green for the scoped variant;
  credential scope-layer re-homed with fail-closed/zeroize regression tests.
- **P4 — Engine/api/core rewire.** Engine, api, core migrated to the scoped
  port; per-repo error enums removed; knife/lease/loom ported with
  equivalence notes; §12.2/§11.x invariants re-verified.
- **P5 — Identity zoo.** `User/Org/Workspace/Membership/Resource/Trigger/
  Quota/Audit/Blob` stores × 3 backends. Independent repos →
  parallelizable across implement-workers; does **not** block P4.
- **P6 — Migration consolidation.** Delete flat legacy tree; structured
  per-backend trees canonical; runner rewired; `task db:reset` documented.
- **P7 — Docs/ADR.** ADR-0068 (supersede the deferral; record §2 decisions),
  README, MATURITY.md, ENGINE_GUARANTEES.md, AGENTS.md layer map, deny.toml
  comments, `pool.rs` removed if still unused.

Big refactors run in one pass within a phase; the compiler is trusted at
phase boundaries, not every few edits. Phases P2/P5 fan out to parallel
agents where repos are independent.

## 12. Assumptions & Non-goals

**Assumptions:** no deployed prod DB (clean migration cutover; dev DB
destroyed, `task db:reset`); the spec-review gate was delegated to the
implementer (rigorous self-review substitutes; ADR-0068 records this);
`nebula-core` ID newtypes are stable and reused (port does not re-define IDs).

**Non-goals:** not the execution state machine (`nebula-execution` owns FSM
legality); not the engine orchestrator; cross-replica *lease* coordination
beyond ADR-0041 refresh-claim (canon defers to 1.1); Redis/S3 stay
experimental KV/blob, not execution state; a dedicated `apps/server`
composition root remains a separate follow-up (the knife test stays the
canonical end-to-end wiring).

## 13. Companion ADR & doc updates

- **ADR-0068** — `docs/adr/0068-nebula-storage-spec16-port-adapter-tenancy.md`:
  supersedes the "Sprint E deferred" stance; records object-safe trait-shape
  decision + rationale, crate split + layer-map/deny.toml deltas, data-vs-policy
  multi-tenancy, `TransitionBatch` unit-of-work + lease fencing, migration
  consolidation + cutover assumption, SQLite parity boundary, the
  delegated-review note.
- Updates: `crates/storage/README.md`, `crates/storage-port/README.md` (new),
  `crates/tenancy/README.md` (new), `docs/MATURITY.md` (storage row → stable;
  add port/tenancy rows), `docs/ENGINE_GUARANTEES.md` (durability matrix:
  SQLite now first-class for the core), `AGENTS.md` (layer map + workspace
  layout), `deny.toml` (wrappers + comments).
