---
name: nebula-storage
role: Storage Port (Repository Implementations + CAS + Outbox)
status: partial
last-reviewed: 2026-07-22
canon-invariants: [L2-11.1, L2-11.3, L2-11.5, L2-12.2, L2-12.3]
related: [nebula-execution, nebula-engine, nebula-core, nebula-error]
---

# nebula-storage

## Purpose

A durable workflow engine needs a persistence seam that the engine and API can drive without
coupling to a specific database. More critically, it needs a place where optimistic CAS state
transitions, journal appends, and outbox writes can share the same logical operation — the
"two truths" anti-pattern (canon §14) forbids splitting those writes across separate
transactions. `nebula-storage` is that seam: it implements the spec-16 storage port for
execution state, workflow definitions and versions, the append-only journal, idempotency keys,
checkpoints, leases, identity stores, owner-bound credential persistence, and the durable
control-queue outbox. SQLite and PostgreSQL are deployment backends; InMemory implementations are
internal test/reference/conformance adapters only.

## Role

*Storage Port.* Implements the object-safe `nebula-storage-port` traits with Optimistic
Concurrency Control (DDIA ch 7) on `ExecutionStore::commit` and a Transactional Outbox (EIP
"Guaranteed Delivery", DDIA ch 11) via `TransitionBatch` + `ControlQueue`. Provides the
single persistence layer the knife scenario (canon §13) exercises end-to-end.

## Public API

The contract is the spec-16 storage **port** in `nebula-storage-port`
(`ExecutionStore` + the atomic `TransitionBatch`, `ExecutionJournalReader`,
`NodeResultStore`, `CheckpointStore`, `IdempotencyGuard` /
`IdempotencyStore`, `WorkflowStore` / `WorkflowVersionStore`,
`ControlQueue`, `WebhookActivationStore`, `RefreshClaimStore`, and the
identity-zoo stores; owner-bound `CredentialPersistence`; `StorageError`;
the plain-data `Scope`). This crate
provides the adapters:

- `inmem::*` — internal test/reference/conformance adapters and the loom probe;
  not a supported deployment backend.
- `sqlite::*` (feature `sqlite`) — single-writer-correct adapters over a
  port-scoped schema; `init_schema` installs it for `:memory:` / test
  pools.
- `postgres::*` (feature `postgres`) — production multi-process adapters
  (real tx + `FOR UPDATE SKIP LOCKED`) over the same port-scoped schema.
- `repos::*` — the non-port backend traits that still have live
  consumers: `ControlQueueRepo` (+ `InMemoryControlQueueRepo`,
  `pg::PgControlQueueRepo`), `IdempotencyStoreRepo`,
  `WebhookActivationRepo`, and the identity-row glue the Postgres
  backend implements.
- `pg::PgOAuthLoginFinalizer` (feature `postgres`) plus the
  `repos::OAuthLoginFinalize*` command/outcome types — the technical,
  storage-owned Plane-A completion seam. Each call receives already-verified
  identity inputs and performs no provider network I/O. An existing
  `(provider, subject)` link is authoritative; same-subject races converge
  internally. For a new subject, an unused verified email may create a new
  OAuth-only user, while an email owned by another account rolls back with
  `AccountLinkRequired` and never auto-links. The finalizer atomically records
  either user/link/session or, for an MFA-enabled linked user, an opaque
  challenge plus MFA-required outcome with no session. Provider codes and
  tokens never enter this contract.
- Plane-A OAuth-state admission has a hard bound of 10,000 live rows per shared
  PostgreSQL deployment. Capacity check plus insert is one fail-closed admission
  operation; a full or contended gate returns the capacity outcome used by the
  API's 429 response and writes no state. The Memory backend enforces the same
  numerical bound process-locally in `nebula-api`.
- `repos::MfaEnrollmentRepo` and `pg::PgMfaEnrollmentRepo` own the Plane-A MFA
  replacement boundary. One expiring candidate exists per user, separate from
  the active factor. Start replaces only that candidate; confirmation consumes
  the exact live candidate and installs it atomically, so replay/concurrent
  losers cannot disable or overwrite active MFA. Secret bytes remain an opaque
  envelope at this contract.
- Browser-session repositories accept the one-time presented cookie separately
  from `SessionDraft` and persist only
  `SHA-256("nebula:plane-a:session-cookie:v1\\0" || token)`. Migration `0038`
  intentionally truncates pre-digest sessions; raw cookies cannot be migrated
  without preserving the bearer authority.
- `identity_secret::IdentitySecretCodec` stores active and pending TOTP seeds
  as `EncryptedData` v1 AES-256-GCM envelopes. AAD binds the exact 16-byte user
  id plus a distinct active/pending purpose, so promotion must decrypt the
  verified candidate and re-seal it; copying ciphertext between columns cannot
  grant authority. The codec and credential encryption consume one atomic
  `KeyProvider::current()` snapshot (`key_id` + key) so a live KMS rotation
  cannot pair metadata from one generation with bytes from another.
- `pg::PgIdentitySecretMigrator` is a startup data migrator, not a schema
  migrator: all DDL stays in numbered migration `0038`. It uses a cancellation-
  safe retired advisory-lock connection, bounded reads, equality-guarded CAS,
  user-version fencing, explicit old-key rotation, and repeated verification;
  the Postgres auth backend is not exposed until convergence succeeds.
- crate-local `StorageError`, plus `StorageFormat`
  (serialization format abstraction).

Applied migrations `0001..0038` are immutable SQLx-checksummed history.
Credential lifecycle migration
`0039_credentials_owner_and_record_state.sql` is paired across SQLite and
PostgreSQL. It makes owner identity and structural live/tombstoned state
database invariants, preserves valid live material exactly, and converts
legacy rows carrying a top-level `revoked_at` key into secret-free terminal
records without version inflation. The same migration adds nullable
`claim_id` incident identity to historical sentinel evidence and a global
partial unique index; newly accounted incidents always carry the UUID while
pre-0039 rows remain `NULL`.

Paired migration `0040_credential_refresh_retry_gate.sql` adds the closed
structural refresh-retry gate and backend-authored material epoch. Creates and
migrated rows start at `CredentialMaterialEpoch::MIN` with a clear gate. Every
replacement carries an outer `CredentialMaterialTransition`:
`Preserve { refresh_retry }` retains the epoch while applying an explicit
preserve/clear/permanent/timed gate transition; `Advance` increments the epoch
and unconditionally clears the old gate. Epoch overflow fails closed.
Tombstones clear the gate. Timed deadlines and admission use the authoritative
backend clock (`clock_timestamp()` after PostgreSQL lock waits; millisecond UTC
samples in SQLite), and remaining delays are rounded up to bounded whole
seconds. Unknown mode, phase, kind, or diagnostic encodings fail closed as
corrupt records. A permanent (`Never`) gate is scoped to the current
credential-material epoch, not the credential identity. Reauthentication
and material replacement/reconnect both use `Advance`; reauthentication also
blocks through its durable flag.
`refresh_retry_snapshot` reads version, material epoch, reauthentication state,
gate, and backend clock in one statement/snapshot so rechecks cannot combine
observations from different aggregate versions. The gate and epoch never enter
user metadata.

Credential adapters are constructible only through their ready-store
constructors. Those constructors hold a backend-appropriate bounded startup
lock across read-only schema admission, canonical migration, and postflight;
unsupported, forged, ownerless, or corrupt schemas fail unchanged. Raw pools
cannot bypass admission. Runtime authority comes only from the mandatory typed
selector and owner column; metadata never grants access.
`SqliteCredentialPersistence::refresh_claim_repo()` and
`PgCredentialPersistence::refresh_claim_repo()` are the curated composition
seams for durable refresh coordination. Each clones its admitted private pool
without exposing raw SQL authority, so credential rows, claim poison, and
sentinel evidence share one backend-specific schema lifecycle and database.

Execution / workflow persistence goes through the port adapters; the
legacy `ExecutionRepo` / `WorkflowRepo` / `Pg*Repo` surface and the
never-implemented spec-16 trait placeholders were deleted (ADR-0072).

Credential coordination — durable refresh claim (П2 / ADR-0041):

- `credential::refresh_claim::RefreshClaimRepo` — cross-replica claim seam for the engine's
  two-tier `RefreshCoordinator` (L1 in-process coalescer + L2 durable claim). Provides
  CAS-based `try_claim` (one acquirer wins under contention), `heartbeat` (TTL extension
  validated against `ClaimToken` generation), idempotent `release`, and `reclaim_stuck`
  (deletes expired Normal rows, but atomically records and retains expired in-flight rows as
  durable poison). `mark_sentinel` flags an in-flight IdP POST so the sweep can account a
  mid-refresh crash exactly once by the globally unique claim UUID;
  `count_sentinel_events_in_window` backs the engine's
  N=3 distinct, explicitly reconciled incidents in 1h `ReauthRequired` observation. Repeated
  requests and sweeps against one poisoned claim count once; N is an escalation signal, never a
  provider retry budget. The count uses the
  database clock rather than replica wall clocks, per sub-spec
  credential refresh coordination design (design records are maintained in the maintainers' private design vault, not in this public repository)
  §3.4-§3.6.
- `RefreshClaim`, `ClaimAttempt`, `ClaimToken`, `RepoError`, `HeartbeatError`,
  `ExpiredClaim`, `SentinelState`, `ReplicaId` — DTO surface re-exported at
  `nebula_storage::{RefreshClaim, ClaimAttempt, ClaimToken, …}`.
- `InMemoryRefreshClaimRepo` — internal reference implementation for tests and
  conformance; not a supported single-replica deployment backend.
- Feature `sqlite` adds `SqliteRefreshClaimRepo` (default local backend; `SQLITE` migrations
  `0022_credential_refresh_claims` + `0023_credential_sentinel_events` + the incident-key
  extension in `0039_credentials_owner_and_record_state`).
- Feature `postgres` adds `PgRefreshClaimRepo` (production multi-replica backend; `POSTGRES`
  migrations `0022_credential_refresh_claims` + `0023_credential_sentinel_events` + the
  incident-key extension in `0039_credentials_owner_and_record_state`).

## Contract

- **[L2-§11.1]** `ExecutionStore::commit` is the **single source of truth** for execution
  state. Applies CAS on `version` and gates every transition with the lease `FencingToken`.
  If persistence is unavailable, the operation fails — it does not silently mutate in-memory
  state. Seam: `crates/storage-port/src/store/execution.rs`.

- **[L2-§11.3]** Idempotency enforcement lives here via the port `IdempotencyGuard` /
  `IdempotencyStore`. Key shape `{execution_id}:{node_id}:{attempt}` is defined in
  `nebula-execution`; the adapter folds scope into storage so callers cannot share keys across
  tenants. Seam: `crates/storage-port/src/store/idempotency.rs`.

- **[L2-§11.5]** `TransitionBatch::journal` backs the durable `port_execution_journal`
  (append-only, replayable) and is committed with the state transition. `CheckpointStore`
  remains **best-effort**: a checkpoint write failure may log and not abort execution; work
  since the last checkpoint may be replayed or lost. Seams:
  `crates/storage-port/src/batch.rs` and `crates/storage-port/src/store/checkpoint.rs`.

- **[ADR-0009]** Resume-persistence schema foundation. `ExecutionRepo::set_workflow_input` /
  `get_workflow_input` persist the workflow trigger payload alongside the execution row
  (issue #311). `save_node_result` / `load_node_result` / `load_all_results` persist the full
  `ActionResult<Value>` variant per node attempt (issue #299) so resume can replay edge
  decisions through `evaluate_edge` (foundation for #324, #336). `NodeResultRecord` carries a
  `schema_version`; an unknown version surfaces as `ExecutionRepoError::UnknownSchemaVersion`
  rather than a silent fall-back. Engine consumers land in downstream chips B2 / B3 / B4.

- **[L2-§12.2]** The `execution_control_queue` outbox is written in the **same logical
  operation** as the state transition it accompanies. Cancel signals must be enqueued atomically
  with the `cancelling` transition. A handler that transitions state without enqueueing, or
  enqueues without transitioning, violates this invariant.

- **[L2-§12.3]** The default local storage path is **SQLite** (file or `sqlite::memory:`).
  In-process tests use `nebula_storage::test_support` (`sqlite_memory_*` helpers), not a
  separate HashMap "memory backend." There is **one** local storage path.

- **[ADR-0041 / sub-spec §3]** `RefreshClaimRepo::try_claim` MUST be atomic under
  contention — exactly one of N concurrent acquirers across N replicas wins. Implementations
  achieve this via a CAS-shaped `INSERT … ON CONFLICT DO UPDATE WHERE expires_at < now()
  AND sentinel = Normal` predicate (Postgres + SQLite) or a per-key `parking_lot::Mutex`
  guarded `HashMap` swap (in-memory). An expired `RefreshInFlight` row is a durable
  `OutcomeUnknown`: `try_claim` must never reset it or start provider egress. `heartbeat` and
  `mark_sentinel` MUST validate the unexpired `ClaimToken`. `reclaim_stuck` atomically records
  the poisoned generation's sentinel event exactly once while retaining the row; only expired
  Normal rows are deleted. Event deduplication is keyed by the claim UUID, never by a reusable
  generation, holder, or timestamp. Explicit owner-qualified reconciliation is the sole future
  path that may clear poison. Seam: `crates/storage/src/credential/refresh_claim/`.

## Non-goals

- Not the execution state machine — see `nebula-execution` (state types, transition legality).
- Not the engine orchestrator — see `nebula-engine` (drives the port `ExecutionStore`).
- Not an action dispatcher — see `nebula-runtime`.
- Not a KV cache (Redis) as a production execution backend — Redis feature is KV only, not
  execution state.

## Maturity

See `docs/MATURITY.md` row for `nebula-storage`.

- API stability: `stable` — the single architecture is the spec-16
  storage **port** (`nebula-storage-port`), implemented here for
  InMemory + SQLite + Postgres and rewired through `engine` / `api`
  (ADR-0072). The legacy `ExecutionRepo` / `WorkflowRepo` dual layer was
  deleted.
- Lease fencing is **enforced**: `acquire_lease` returns a monotone
  `FencingToken` that gates every committed `TransitionBatch`, so a
  superseded holder is rejected even on a matching CAS version (the
  zombie-runner hole; ADR-0072). Verified by
  `crates/engine/tests/lease_takeover.rs`, the lease-handoff loom probe
  at `crates/storage-loom-probe/src/lease_handoff.rs`, and the
  conformance matrix's lease cases.
- The retained `repos::*` surface (`ControlQueueRepo`,
  `IdempotencyStoreRepo`, `WebhookActivationRepo`, identity-row glue)
  keeps live consumers (the API idempotency middleware and the Postgres
  glue) and is no longer "planned spec-16".
- S3 and Redis features are optional and experimental; local filesystem
  backend is `planned`.
- Postgres adapter + identity stores are compile-verified and structurally
  identical to the runtime-verified SQLite tree, but Postgres runtime
  coverage is `DATABASE_URL`-gated and skip-clean — not claimed as
  pg-verified (ADR-0072 "Verification status").
- The PostgreSQL OAuth finalizer additionally has a real-Postgres concurrency
  and rollback suite covering same-subject convergence, shared-email
  `AccountLinkRequired`, different-email creation, existing/soft-deleted/
  malformed links, MFA challenge-without-session, and session-insert rollback.
  This verifies the narrow finalizer transaction; it does not merge the earlier
  atomic state consume or provider egress into that transaction and does not
  change the broader adapter verification claim.

## Database migrations

Migrations live in two per-backend trees: `migrations/postgres/` and
`migrations/sqlite/` (logically identical tables; dialect types differ).
There is no flat top-level migration tree.

The spec-16 storage-port adapters persist through the `port_*` tables in
`0027_port_adapter_schema.sql`, which is byte-identical to the embedded
`src/{postgres,sqlite}/schema.sql` that `init_schema` applies for
in-memory / test pools. The migration is the canonical source for a real
database rebuild; the embedded schema is the test/`:memory:` path. Keep
the pair in lockstep (regenerate the migration with `cp` from the
embedded schema — see the per-tree README).

`task db:migrate` applies pending Postgres migrations
(`--source crates/storage/migrations/postgres`, `DATABASE_URL`-gated).
`task db:reset` **drops and recreates the database** then re-runs every
migration — it destroys all local dev data.

### Plane-A identity migration retention

Migration `0038` encrypts the live TOTP column in place, but an `UPDATE` does
not erase historical plaintext from PostgreSQL WAL, dead tuples, replicas,
snapshots, or pre-migration backups. Likewise, backups containing old-key
envelopes remain coupled to that key. Operators must quarantine and expire
pre-migration backups/WAL according to their retention policy, retain every
decrypt-only legacy key until all backups that require it have expired, and
test restore before retiring a key. Restoring an old snapshot requires running
the same startup convergence before serving authentication traffic.

Strict environments that cannot accept that historical-retention window must
invalidate affected MFA factors and require re-enrollment; live-row
convergence must not be described as forensic erasure. The built-in server
resolves only the current `NEBULA_CRED_MASTER_KEY`; old-key recovery requires
an explicit composition using `IdentitySecretCodec::with_legacy_keys` until a
reviewed first-party legacy-key configuration surface ships.

## Related

- Canon: `docs/PRODUCT_CANON.md` §11.1, §11.3, §11.5, §12.2, §12.3.
- ADR-0072 (storage spec-16 port / adapter / tenancy), kept in the maintainers' private design vault
  (port / adapter / tenancy decision, supersession, the three
  correctness bugs, the migration-gap history).
- Siblings: `nebula-storage-port` (the port contract), `nebula-tenancy`
  (decorators for general Scope-taking ports), `nebula-execution` (state types),
  `nebula-engine` (transitions via the port `ExecutionStore` +
  `TransitionBatch`), `nebula-core` (ID types).

## Appendix

### Single storage architecture — the spec-16 port (ADR-0072)

There is one architecture: the spec-16 storage **port**
(`nebula-storage-port`, Core tier — ISP-segregated object-safe traits,
port-local DTO rows, `StorageError`, the atomic `TransitionBatch`, the
plain-data `Scope`). This crate implements the general ports for **InMemory +
SQLite + Postgres**. Credential persistence uses an explicitly test-only
**Reference** adapter plus SQLite/Postgres; Reference is
semantic/conformance-only and never a deployment backend.
`nebula-tenancy` wraps the general Scope-taking ports with scope-enforcing
decorators, while credential persistence is directly owner-bound by
`CredentialOwner` / `CredentialSelector`. `engine` / `credential` / `api` /
first-party apps consume the technical port. The legacy
`ExecutionRepo` / `WorkflowRepo` dual layer and the never-implemented
`repos::{execution,workflow,execution_node,journal}` placeholders were
deleted.

The retained `repos::*` traits (`ControlQueueRepo`,
`IdempotencyStoreRepo`, `WebhookActivationRepo`, and the identity-row
glue the Postgres backend implements) are not part of the deleted dual
model — they keep live consumers (the API idempotency middleware, the
`pg::*` glue) and persist through the same per-backend schema.

### Persistence durability matrix (reference from §11.5)

| Artifact | Status | Notes |
|---|---|---|
| `port_executions` row + state JSON | **Durable** (CAS via `ExecutionStore` + `TransitionBatch`) | Source of truth |
| `port_execution_journal` (append-only) | **Durable** | Replayable history; appended in the same commit as state |
| `port_control_queue` (outbox) | **Durable** | At-least-once cancel/dispatch; written in the same `TransitionBatch` (§12.2) |
| stateful checkpoints | **Best-effort** | Write failure logs, does not abort; may replay |
| lease holder / expiry + `fencing_generation` | **Durable + enforced** (ADR-0072) | `acquire_lease` → `FencingToken`; a superseded holder is rejected even on a matching CAS version. Verified by `crates/engine/tests/lease_takeover.rs`, the loom probe at `crates/storage-loom-probe/src/lease_handoff.rs`, and the conformance lease cases |
| idempotency dedup | **Durable** | First-writer-wins via the port `IdempotencyGuard` / `IdempotencyStore`; sweep drives `evict_expired`. Verified by the conformance matrix + `crates/storage/tests/pg_idempotency.rs` (`DATABASE_URL`-gated) |
| In-process `mpsc` / channels | **Ephemeral** | Never authoritative |

### Supported backends

| Backend | Feature flag | Status |
|---|---|---|
| SQLite (file or `sqlite::memory:`) | `sqlite` | `implemented` — local + test default; feature-gated since the wave-2 review (driver footprint not unconditional) |
| PostgreSQL | `postgres` | `implemented` — production path |
| Redis | `redis` | `experimental` — KV only, not execution state |
| S3 / MinIO | `s3` | `experimental` — blob storage |
| Local filesystem | — | `planned` |
