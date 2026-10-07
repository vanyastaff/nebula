# nebula-storage — Agent orientation
> Local guide for `crates/storage/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** The sole adapter implementation of the spec-16 `nebula-storage-port` contract — SQLite/PostgreSQL deployment backends plus internal in-memory test/reference adapters, including owner-bound credential persistence.
**Layer:** Exec — depends only downward (root AGENTS.md -> Layered Dependency Map).

## Common Tasks

| Task | Steps |
|------|-------|
| Add a new port method | Define in `nebula-storage-port`, update all applicable backends and policy decorators, then shared conformance tests. Credential adapters/decorators live under `src/credential/`; general adapters live under `src/inmem/`, `src/sqlite/`, `src/postgres/`. Add paired migrations when needed. |
| Add a port store | 1. Port: trait in `nebula-storage-port/src/store/<aggregate>.rs`, records in `dto/<aggregate>.rs` (see that crate's AGENTS.md). 2. Paired migrations. 3. One file named `<aggregate>.rs` in **each** of `src/inmem/`, `src/sqlite/`, `src/postgres/`, re-exported from the backend's `mod.rs` as `InMemory*` / `Sqlite*` / `Pg*`. 4. In each SQL file one `fn decode_<dto>(row) -> Result<Dto, StorageError>`; every query maps errors through `sql_error`. 5. Tenancy decorator for a Scope-taking store, and its classification in `nebula-tenancy`. 6. Cases in the shared conformance suite run against all three backends. |
| Add a SQL migration | Create paired `migrations/{postgres,sqlite}/NNNN_description.sql` files when the logical schema is shared. Numbered SQLx migrations are the sole setup source; never add a `src/**/schema.sql` snapshot. Classify the migration as aggregate-neutral or aggregate-transforming before changing the executable catalog-boundary test. Run the curated `task db:migrate` operator; never run raw SQLx migration against a non-empty database. |

## Commands

- `cargo check -p nebula-storage --features sqlite,postgres` — compile both deployment backends; default features alone compile neither SQL backend.
- `cargo nextest run -p nebula-storage --features sqlite --test conformance` — exercises the SQLite/reference matrix; the PostgreSQL cases remain unverified without their prerequisites.
- With `DATABASE_URL` set to a disposable test database: `NEBULA_REQUIRE_POSTGRES=1 cargo nextest run -p nebula-storage --features sqlite,postgres --test conformance`. This harness fails on missing PostgreSQL prerequisites. Other suites, such as `pg_idempotency`, still return early when the URL is absent; do not generalize the strict flag to every test.
- Migrations: per-backend ordered trees `migrations/{postgres,sqlite}/`; both production startup
  and test/`:memory:` setup run those exact catalogs. Existing numbered files are immutable.
  `task db:migrate` uses the admitted server-owned operator. `task db:reset` is the only raw-run
  path and is safe only because its prompt-protected sequence first drops and recreates the
  database.
- `CatalogOnly` may automatically cross a pending migration only after review proves that the
  migration is aggregate-neutral. An aggregate-transforming or destructive migration needs its
  owner's preflight and postflight under the same setup guard/session, or the general catalog
  floor must move to a head at which that transformation is already known safe. The executable
  head/floor pin intentionally fails when a new migration is added; never advance it as a
  mechanical catalog update.

## Key files

- `src/lib.rs` — module/feature map and adapter re-exports (`InMemory*`, `StorageError`).
- `src/inmem/` — internal test/reference/conformance adapters and loom probes; not a supported deployment backend.
- `src/sqlite/` · `src/postgres/` — feature-gated port adapters over the port-scoped schema (Postgres uses real tx + `FOR UPDATE SKIP LOCKED`).
- `src/auth/` — Plane-A account persistence (users, sessions, PATs, OAuth state, external identities, MFA, identity secrets, session-token digests): traits and rows in `auth`, PostgreSQL implementations in `auth/postgres/`. Outside the port contract by design.
- `src/http_idempotency/` — the API's idempotent-replay response cache (`IdempotencyStoreRepo`, `PgHttpIdempotencyStore`); not the port's per-attempt `IdempotencyStore`.
- `src/webhook_activation.rs` — the webhook activation spec persisted in `triggers.config`.
- `src/sql_error.rs` — the one `sqlx::Error` → `StorageError` classification (value-free; dialect chosen by error type) plus `decode_u64` / `decode_i32` / `encode_u64`. Every SQL adapter maps errors through it except `*/resource_runtime.rs`, which still has its own classifier (known debt: its `corrupt()` mixes corrupt data, counter exhaustion and caller TTL).
- `src/auth/postgres/oauth_login.rs` + `src/auth/oauth_login.rs` — storage-owned Plane-A
  OAuth finalization: every call performs no network I/O and atomically records
  either user/stable-link/session or an MFA challenge-without-session outcome.
  A subject-only call may roll back as `VerifiedEmailRequired` before optional
  verified-email egress; the later finalizer call rechecks all races.
- `src/credential/refresh_claim/` — ADR-0041 CAS refresh-claim repo (`try_claim`/`heartbeat`/`release`/`reclaim_stuck`); in_memory + sqlite + postgres.
- `src/credential/layer/` — encryption / audit / cache decorators around credential persistence.
- `src/credential/{sqlite,postgres}.rs` — ready-store deployment adapters for the owner-bound
  `CredentialPersistence` port. Migration `0070` files each credential under the workspace its
  `CredentialOwner` names (`org_id`, `workspace_id`; a partition naming no workspace owns no row)
  in the deployment database beside tenancy: credentials and pending flows cascade from their
  workspace (a create requires it live), claims and incidents from their credential, and archived
  (`deleted_at`) credentials are hidden from every read, write, claim and adjudication.
  Compositions open the store with `connect_pool` on the deployment pool their execution stores
  use (admission and migration still run; no second pool, no credential pool size); tests
  provision tenants through a tenancy store on that same pool. `sqlite::open_memory_deployment`
  is the in-memory deployment database. `InMemoryCredentialPersistence` (feature
  `credential-in-memory`) is the reference adapter for tests without a deployment database.

## Adapter rules (every backend, every new or touched store)

- **Layout.** A new port store gets one file with the same name in `inmem/`, `sqlite/`
  and `postgres/`. Existing exceptions: the journal reader lives in `inmem/journal.rs` but
  `*/control_queue.rs` in SQL, `inmem/node_result.rs` has no SQL twin, `postgres/rate_limit.rs`
  is PostgreSQL-only by design, and credential persistence lives in `src/credential/`. When a file grows to hold several aggregates, it becomes a directory
  (`identity/`): `mod.rs` keeps the shared decoders and helpers, one file per aggregate.
  Code that is not a port adapter goes in a module named for what it is (`auth/`,
  `http_idempotency/`), never in a backend-named tree.
- **Errors.** Every `sqlx::Error` goes through `sql_error::storage_error` (or
  `storage_error_for(entity, _)` when callers branch on which record collided). Choose the
  variant by what failed: unreachable/busy → `Connection`; stored data that does not decode
  (NULL in a NOT NULL column, unknown enum text, out-of-range integer, bad JSON) →
  `Corrupt`; caller passed something invalid → `InvalidInput`; deployment misconfiguration
  → `Configuration`; a broken internal invariant → `Internal`. Never `Connection` for bad
  data and never a default value in its place.
- **Value-free messages.** Errors name the entity, column, constraint or SQLSTATE — never
  a stored or submitted value (emails, slugs, payloads, tokens).
- **Decoding.** One `decode_<dto>` per DTO per backend, in the module that owns the table.
  Nullable columns decode as `Option<T>` and propagate errors — never `.ok()` or
  `unwrap_or_default()`. SQLite returns `0`/`""` for NULL scalars, so NOT NULL columns are
  read as `Option<T>` and rejected when `None` (or the column is declared NOT NULL in every
  migration). Integers cross the signed SQL boundary only through `decode_u64` /
  `decode_i32` / `encode_u64` — never `as`. Older adapters (`execution`, `workflow`,
  `control_queue`, `job_dispatch`, `turn_handoff`) still bind with `as i64`: convert them
  when you touch them.
- **Reference adapter.** `inmem/` returns the same variants, order and uniqueness outcomes
  as the SQL backends; conformance asserts it.
- **Tests.** Behaviour shared by backends is a conformance case in `tests/`, run against
  all three. Unit tests of private helpers live beside them: inline `mod tests` when short,
  otherwise a module directory's `mod.rs` declares `#[cfg(test)] mod <name>_tests;`
  (e.g. `sqlite/identity/decoder_tests.rs`) — no new `#[path]` includes.

## Conventions & never-do

- `ExecutionStore::commit` is the single source of truth: CAS on `version` + lease `FencingToken` gating; if persistence is unavailable it FAILS — never silently mutate in-memory state.
- Outbox atomicity (§12.2): when a transition produces control messages, include them in the SAME `TransitionBatch` as the state change. Do not split that intent into a later enqueue. An empty outbox is valid for transitions that produce no control message.
- `try_claim` must be atomic under contention (exactly one winner of N replicas). It may replace
  only an expired `Normal` row. An expired `RefreshInFlight` row is durable
  `OutcomeUnknown` poison: reclaim atomically records its event exactly once but never deletes
  the row, and no caller may retry provider egress. The event's incident identity is the globally
  unique claim UUID; holder, generation, and timestamps are observability fields, not dedup keys.
  `heartbeat` and `mark_sentinel` require an unexpired, generation-matching claim.
- Plane-A OAuth completion has separate boundaries: atomic state consume,
  provider egress, then a short storage-owned finalizer. An existing
  `(provider, subject)` link is authoritative and same-subject races converge.
  An email collision without that link is the deliberate
  `AccountLinkRequired` outcome: roll back, create no session, and never
  auto-link. For an MFA-enabled linked user, challenge + MFA-required outcome
  commit atomically with no session. Never perform provider network I/O while a
  finalizer transaction holds locks.
- Plane-A OAuth-state admission is hard-capped at 10,000 live rows per shared
  PostgreSQL deployment. Capacity check and insert must share one serialization
  point; full or contended admission fails closed, writes no state, and maps to
  HTTP 429. Do not replace it with an approximate count-then-insert sequence.
- Pending MFA enrollment is separate from the active user factor. Starting an
  enrollment may replace only the expiring candidate; installing a verified
  candidate must consume the exact live candidate and update the active secret
  in one transaction. Replays, replacements, expiry, and concurrent losers do
  not modify active MFA state.
- This crate is NOT the state machine (`nebula-execution`), orchestrator (`nebula-engine`), or tenant-scope policy owner. `nebula-tenancy` wraps the general Scope-taking adapters; credential persistence is the deliberate owner-bound exception. Do NOT re-add the deleted legacy `ExecutionRepo`/`WorkflowRepo` surface (ADR-0072).
- Every credential predicate is owner-bound (`CredentialSelector` or `CredentialOwner`); wrong-owner and missing are indistinguishable. Owner metadata is compatibility/audit data only and never grants authority.
- The credential refresh-retry gate and material epoch are structural row state, separate from metadata and refresh-claim TTL. Backends author epochs: create/migration starts at `CredentialMaterialEpoch::MIN`; `CredentialMaterialTransition::Preserve { refresh_retry }` retains the epoch and applies its explicit gate transition; `Advance` increments the epoch and unconditionally clears the old gate; overflow fails closed. SQLite/PostgreSQL compute `SetAfter` and admission from their own wall clock (PostgreSQL uses `clock_timestamp()` after lock waits); unknown codecs fail closed, and tombstones carry no gate.
- Replacement writes the material columns (`data`, `state_kind`, `state_version`, `expires_at`) only for `Advance { Replace }`; `EncryptionLayer` seals only that material, so key rotation happens on material writes alone. The admission epoch (migration 0061) advances in the same transaction as each closing write: replace (`CredentialReplacement::advances_admission_epoch`), a won revoke CAS, `mark_sentinel`, and threshold escalation — claim row before credential row, never touching `version`/`updated_at`. Every status projection selects it; creates write 1. The in-memory claim repo cannot bump it and is not a deployment backend.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Shared-resource subscription/fanout/handoff runtime | `resource_fanout_conformance_{inmem,sqlite,postgres}`; SQL fixtures retain their pools for test-only backdating/failpoints, while the InMemory suite uses an injected manual clock. PostgreSQL runs with `NEBULA_REQUIRE_POSTGRES=1`. |
| Port behavior and tenancy | [conformance](tests/conformance.rs) and [identity_conformance](tests/identity_conformance.rs) for the affected backends; report which backend cases actually executed. |
| Migration admission | [schema_source_authority](tests/schema_source_authority.rs), [credential_migration_catalog](tests/credential_migration_catalog.rs), and the SQLite/PostgreSQL schema-admission suites. Review catalog-floor policy before updating expected heads. |
| Execution handoff or remote effects | The `turn_handoff_conformance_*` and `operation_ledger_conformance_*` targets in [tests/](tests/); pair backend evidence with engine/worker ownership tests. |

## See also

- `README.md` — full durability matrix + backend status table.
- ADR-0072 (port/adapter/tenancy); ADR-0041 (refresh claim); [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §11.1/§11.3/§11.5/§12.2/§12.3.
