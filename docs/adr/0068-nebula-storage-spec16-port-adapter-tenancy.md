# ADR-0068: nebula-storage spec-16 port / adapter / tenancy

**Status:** Accepted (2026-05-17)
**Tags:** storage, storage-port, tenancy, engine, api, spec-16, supersession

## Context

`nebula-storage` carried two coexisting architectures: a Layer-1
production seam (`ExecutionRepo` / `WorkflowRepo` + `InMemory*` /
`Pg*` impls) and a Layer-2 "planned / experimental" `repos::*` spec-16
trait set with zero implementations. The engine and API consumed
Layer-1; the spec-16 row model (mandatory multi-tenancy, split workflow
versioning, an atomic state+outbox+journal unit-of-work) was deferred to
"Sprint E — adopt spec-16 row model" in the workspace health-audit spec
("Out of scope for 1.0").

The dual architecture was a standing liability: the engine could not be
tested without sqlx, tenancy was an ad-hoc credential-layer concern,
the §12.2 outbox atomicity was implicit, and lease handoff had a
zombie-runner hole (a superseded holder whose CAS version still matched
could still commit).

The redesign (`docs/superpowers/specs/2026-05-15-nebula-storage-spec16-redesign-design.md`)
replaces the dual model with one object-safe storage port, a
multi-backend adapter, and a tenancy security decorator, then rewires
engine/api/core onto the port. Breaking changes were sanctioned.

## Decision

Adopt the spec-16 row model as the single storage architecture,
decomposed into port + adapter + tenancy crates. The eight §2 decisions:

1. **Object-safe trait family.** The dyn-consumed ports use
   `#[async_trait]` — a deliberate, documented choice: every port call
   bottoms out in network/disk I/O, so the per-call boxed-future
   allocation is noise; `trait_variant` / `dynosaur` would add machinery
   for zero gain on an I/O-bound port. Native `async fn` is reserved for
   non-dyn adapter-internal helpers. `RefreshClaimStore` keeps its
   loom-verified shape — no idiom churn on a proven component.
2. **Crate topology.** `nebula-storage-port` (Core: ISP-segregated
   object-safe traits, port-local DTO rows, `StorageError`,
   `TransitionBatch`, the plain-data `Scope { workspace_id, org_id }`
   value type; deps only `nebula-core` + serde family + `async-trait`;
   **no sqlx**). `nebula-storage` (Exec: InMemory + SQLite + Postgres
   adapters, sqlx, migrations, pool, credential layer).
   `nebula-tenancy` (Business: `ScopeResolver` policy + scope-enforcing
   decorators). `engine` / `core` / `api` depend only on the port;
   only composition roots depend on the adapter + tenancy.
3. **Atomic unit-of-work as a value object.** `TransitionBatch` carries
   the §12.2 triple (state CAS + control outbox + journal append) as one
   logical commit; the builder is the only constructor, so a caller
   cannot transition without declaring scope + expected version +
   fencing token.
4. **Lease fencing gates CAS.** `acquire_lease` returns a monotone
   `FencingToken`; the engine threads it into every committed
   `TransitionBatch`, so a superseded holder is rejected even on a
   matching CAS version — the zombie-runner hole is closed end-to-end
   (correctness bug #1 below).
5. **Multi-tenancy is a security boundary.** Rows carry
   `workspace_id` / `org_id`; the port defines the plain-data `Scope`;
   `nebula-tenancy` owns the policy (resolve `Scope` from `Principal`,
   inject, enforce, deny cross-scope). Cross-tenant access returns
   `NotFound` (never another tenant's row — no existence oracle), proven
   by a dedicated cross-tenant-denial conformance suite (spec §6.1).
6. **Port depends only on `nebula-core` + serde.** Port-local DTOs; no
   `ActionResult` dependency (the node-result record is opaque
   `serde_json::Value` + a `kind_tag` + a `schema_version`).
7. **SQLite parity = API + single-writer correctness, NOT concurrency /
   throughput parity.** SQLite `commit` opens `BEGIN IMMEDIATE`; the
   control-queue claim is a single-consumer status flip (documented, not
   a hidden `FOR UPDATE SKIP LOCKED` substitute).
8. **One behavioral conformance suite** × {InMemory, SQLite, Postgres} ×
   the scoped decorator. The loom / knife / lease tests were ported (not
   deleted) onto the port with an invariant-equivalence note; the
   redundant legacy knife engine-consumer variants were dropped because
   the port knife + `crates/engine/tests/control_dispatch.rs` cover the
   same canon §13 surface.

`deny.toml [wrappers]` gained Core-tier `nebula-storage-port` (broad
allowlist) and Business-tier `nebula-tenancy` (api + composition-root /
scoped-conformance dev-deps) entries; `nebula-storage` wrappers were kept
as the composition seam. The `AGENTS.md` layer map + workspace layout
record the two new crates.

## Supersession

| Superseded | By | Note |
|---|---|---|
| "Sprint E — adopt spec-16 row model" deferral (`docs/superpowers/specs/2026-04-16-workspace-health-audit.md`, ROADMAP "Out of scope for 1.0") | This ADR | The spec-16 row model is no longer deferred — it is the single shipped storage architecture. The Layer-1 `ExecutionRepo` / `WorkflowRepo` / `Pg*Repo` surface and the never-implemented Layer-2 `repos::{execution,workflow,execution_node,journal}` placeholders were deleted; `engine`/`api`/knife/loom run on the port. The retained `repos::*` (control-queue, idempotency, webhook-activation, identity glue) keep live consumers and are no longer framed as "planned spec-16". |

## Correctness bugs found and fixed during the migration

The expand-contract migration surfaced three real correctness defects
that the dual architecture had masked:

1. **§12.2 lease-fencing zombie runner.** A runner whose lease was
   reclaimed could still commit a transition if its CAS version happened
   to still match. Fixed structurally: `acquire_lease` returns a
   `FencingToken`; `commit` rejects a non-current token even on a
   version match (decision 4). Verified by the lease-handoff loom probe,
   the engine `lease_takeover` suite, and the conformance matrix.
2. **tokio paused-time `Instant` in lease-expiry tests.** Lease-expiry
   assertions used a time source that did not advance under
   `tokio::time::pause`, so an "expired" lease never actually expired in
   the test clock. Fixed by sourcing expiry from the paused-time-aware
   clock so the test exercises real TTL semantics.
3. **Workflow timestamp serde-format mismatch.** The workflow
   create / update / activate handlers wrote `created_at` / `updated_at`
   into the stored definition as integer Unix seconds, but
   `WorkflowDefinition`'s serde contract encodes those `DateTime<Utc>`
   fields as RFC 3339 strings. `activate` round-trips the stored
   definition through `serde_json::from_str::<WorkflowDefinition>`, so a
   create-then-activate of the *same* workflow failed to parse and
   returned HTTP 400. The legacy knife masked this by seeding the
   activate step through a direct-repo back door with a string-timestamp
   fixture; the scoped-port knife exercises the real create→activate
   flow and exposed it. Fixed: handlers persist the RFC 3339 form
   (consistent with the type's serde); the `WorkflowResponse` API field
   stays Unix seconds, derived from the same instant;
   `extract_timestamp` already accepted both encodings so responses are
   byte-unchanged.

## Migration / cutover history (the storage-port schema gap)

The spec-16 port adapters persist through dedicated `port_*` tables
(`port_executions`, `port_execution_journal`, `port_control_queue`,
`port_idempotency_*`, `port_webhook_activations`, `port_workflows`,
`port_workflow_versions`, and the identity-zoo `port_users` …
`port_blobs`). During the adapter work these existed **only** as the
embedded `src/{postgres,sqlite}/schema.sql` applied via `init_schema`
for in-memory / test pools — **no migration tree created them**. Because
Postgres coverage is `DATABASE_URL`-gated and was never pg-verified in
the worktree, a real `task db:reset && task db:migrate` would have
produced a database the Postgres port adapter could not use. This gap
was flagged honestly in the contract commit and the
`0011_executions.sql` headers, then closed by adding a byte-identical
`0027_port_adapter_schema.sql` to both per-backend migration trees
(`migrations/{postgres,sqlite}/`), deleting the orphaned flat legacy
tree, and repointing the Taskfile at the canonical per-backend source.
The migration and the embedded schema are kept in lockstep (`cp`
regeneration documented in each tree's README).

## Verification status

InMemory and SQLite are runtime-verified (the conformance + identity
matrices run live; the SQLite `port_*` DDL is byte-identical to the
runtime-proven embedded schema). Postgres is **compile-verified and
structurally identical to the SQLite tree but not runtime-applied**
(`DATABASE_URL` unset in the worktree); it is honestly classified
done-but-pg-unverified, never claimed as pg-verified, and skip-clean in
every gated test.

## Consequences

- The engine is testable without sqlx (depends only on the port).
- Tenancy is a first-class, conformance-tested security boundary, not an
  ad-hoc credential-layer concern.
- §12.2 atomicity and lease fencing are structurally enforced, not
  discipline-based.
- SQLite is first-class for the execution core (spec §5 parity boundary
  documented).
- The remaining Postgres runtime verification is the one open
  follow-up, gated on a `DATABASE_URL` environment.

### Known follow-up (out of scope): engine per-execution tenant scoping

The api `state.rs` placeholder-scope hole (coderabbit `3255514540`) is
**closed**: `AppState` stores raw, undecorated port handles, every
accessor takes a `&Scope`, the workflow/execution handlers derive the
real request scope from `TenantContext` via
`nebula_tenancy::request_scope`, and the readiness probe uses a
scope-agnostic `WorkflowStore::is_reachable()`. The api/port/tenancy
tenant boundary is therefore per-request and conformance-tested.

`nebula-engine` independently still calls its storage-port handles with
a fixed internal `engine_scope() = Scope::new("nebula", "nebula")`
placeholder (≈20 call sites across
`crates/engine/src/{engine,control_dispatch,store_seam}.rs`), relying on
the tenancy decorator to substitute the request scope — the *same class*
of latent issue, in the engine rather than the api. This was **not**
raised by any PR-#689 review comment and is **orthogonal** to spec-16
and to the api `state.rs` fix; threading a real per-execution tenant
scope through the engine is a separate, sizeable refactor that belongs
in its own follow-up issue/PR, not this merge. The PR's "tenancy
security boundary" claim is therefore scoped to the api/port/tenancy
surface; engine per-execution scoping is a tracked, deliberately
deferred concern (the integration-test harness binds the engine seam's
store handles to the request scope via tenancy decorators so the seam
stays coherent under the per-request api).
