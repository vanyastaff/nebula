# Durable credential persistence (SQLite + Postgres) — plan

> Status: **locked** (2026-06-10). Executes ADR-0088 **D7** ("delete the dead
> `CredentialRepo`/`CredentialRow`/migrations 0008+0017; durable backends bind to
> `StoredCredential`+`EncryptionLayer`") plus an **A-plus** amendment recorded in
> ADR-0088. Stacked on PR #788 (`refactor/credential-testutil-delete`); branch
> `refactor/credential-inmemory-delete`.

## Problem

Credentials are persisted by an **in-memory-only** `CredentialStore`
(`nebula_storage::credential::InMemoryStore`) — the sole impl. Credentials
**evaporate on restart**, which is why the active-credential-lifecycle moat is
latent in production. Goal: real SQLite + Postgres credential persistence,
n8n-style (encrypted at rest in the DB via the existing `EncryptionLayer`; no
external Vault for now), then stop depending on the in-memory store in
production.

## Decision (A-plus): which model is the durable backend

Two credential persistence models exist in-tree:

- **Model B (rich, n8n-like) — DEAD, delete it.** `CredentialRow` +
  `CredentialRepo` (`storage/src/repos/credential.rs`, `rows/credential.rs`) +
  rich tables `credentials`/`pending_credentials`/`credential_audit` (migrations
  0008/0017). **Zero impls, zero constructors, zero FK dependents** (verified).
  ADR-0088 §Pain-3 + D7 already mandate its deletion and explicitly reject
  reviving `CredentialRow`.
- **Model A (thin) — LIVE, build on it.** `StoredCredential` + `CredentialStore`
  (RPITIT trait in `nebula-credential`) + Encryption/Audit/Cache decorator layers
  **outside** the store. `CredentialService`, the engine lifecycle (refresh /
  rotate / CAS), api, and `nebula_tenancy::CredentialScopeLayer` all consume it.

**Chosen: A-plus** — build the durable backend on Model A, **delete Model B**,
and promote a thin set of **identity columns** so the n8n "named, typed
credential" model is queryable from day one without a future live-secret
JSON→column migration:

- `data` stays an **opaque ciphertext BLOB** — `EncryptionLayer` serializes the
  AES-256-GCM `EncryptedData` envelope (which carries `key_id` for rotation)
  to bytes; the store persists those bytes verbatim and **never** sees plaintext,
  a key-id column, or the envelope shape. (Verified: rotation provenance lives in
  the blob, no per-row key column needed.)
- Promote **`name`**, **`owner_id`**, **`kind`** to columns + a partial
  `UNIQUE(owner_id, name)` index. Everything else n8n-ish (display name, icon,
  description, sharing ACLs) stays in `metadata` JSON until a management UI
  consumer is real.

Rejected: **pure A** (name/type buried in JSON → painful live-secret retrofit
when the credential-management UI lands) and **full C / revive Model B**
(builds sharing/scope columns with no consumer, and Model B's
`(org_id, workspace_id, scope)` triple reopens the scope-format drift D7 closed).

### Contract change (API-GATE)

`StoredCredential` gains **one** field:

```rust
/// User-facing credential name (n8n-style "My Google Account"). `None` for
/// system / unnamed credentials. When `Some`, unique per owner.
pub name: Option<String>,
```

All existing `StoredCredential { .. }` construction sites add `name: None`
(test_helpers, the tenancy test double, api/runtime wiring). `CredentialService`
create path threads an optional name through. This is the only contract change;
the `CredentialStore` trait surface is unchanged.

`owner_id` is **not** a new contract field — the store extracts it from
`metadata["owner_id"]` (the value `nebula_tenancy::CredentialScopeLayer` already
injects via `Scope::credential_owner_id`, ADR-0088 D7) into the indexed column at
write time. `kind` = the existing `credential_key`.

## Schema — migration `0030_credentials_store` (next free number)

`0030` first `DROP TABLE IF EXISTS credentials, pending_credentials,
credential_audit` (Model B's never-populated tables — no data loss, no FK
dependents, `schema.sql` does not reference them), then:

**SQLite** (`migrations/sqlite/0030_credentials_store.sql`):

```sql
CREATE TABLE credentials (
    id              TEXT    NOT NULL PRIMARY KEY,  -- StoredCredential.id
    name            TEXT,                          -- user name; NULL = unnamed
    owner_id        TEXT,                          -- from metadata["owner_id"]; NULL = admin/global
    credential_key  TEXT    NOT NULL,              -- Credential::KEY (the "type")
    state_kind      TEXT    NOT NULL,              -- CredentialState::KIND (list filter)
    state_version   INTEGER NOT NULL,              -- CredentialState::VERSION
    data            BLOB    NOT NULL,              -- OPAQUE ciphertext (EncryptionLayer output)
    version         INTEGER NOT NULL,              -- CAS counter (u64; guard the i64 cast)
    created_at      INTEGER NOT NULL,              -- millis since epoch (UTC)
    updated_at      INTEGER NOT NULL,              -- millis since epoch
    expires_at      INTEGER,                       -- millis; NULL = no expiry
    reauth_required INTEGER NOT NULL DEFAULT 0,    -- bool 0/1
    metadata        TEXT    NOT NULL DEFAULT '{}'  -- JSON: display_name/icon/sharing/tags
);
CREATE UNIQUE INDEX idx_credentials_owner_name
    ON credentials(owner_id, name) WHERE name IS NOT NULL;
CREATE INDEX idx_credentials_state_kind ON credentials(state_kind);
CREATE INDEX idx_credentials_expiring
    ON credentials(expires_at) WHERE expires_at IS NOT NULL;
```

**Postgres** mirror: `data BYTEA`, timestamps `TIMESTAMPTZ` (native — millis-INTEGER
is the SQLite-only workaround for lexicographic text ordering), `version BIGINT`,
`state_version BIGINT` (holds the full `u32` losslessly), `reauth_required BOOLEAN`,
`metadata TEXT` (JSON string, mirroring the SQLite store's row mapping — not
`JSONB`; the store never queries inside `metadata`, so the column type is an
implementation detail and TEXT keeps both backends' row structs identical).

Conventions follow `credential/refresh_claim/{sqlite,postgres}.rs`:
- SQLite millis-INTEGER timestamps (`DateTime::timestamp_millis` / `millis_to_utc`);
  **normalize to millis on BOTH backends AND the in-memory store** so conformance
  `updated_at`-equality assertions don't diverge across impls.
- CAS via `INSERT ... ON CONFLICT(id) DO UPDATE ... WHERE version = :expected
  RETURNING ...`; map `u64 → i64` with an explicit guard (`version <= i64::MAX`),
  **no silent `as` cast**, violation → `StoreError::Backend`.
- `data` column is byte-exact (`BLOB`/`BYTEA`, never `TEXT`); conformance asserts
  byte-identity round-trip of `data`.
- hard delete (matches `InMemoryStore`); no soft-delete column.

Audit durable sink (`SqliteAuditSink`/`PgAuditSink` from `AuditEvent`) is a
**separate optional** deliverable — out of the core store migration.

## Pending-state store — stays ephemeral in-memory for 1.0

`DynPendingStateStore` (OAuth / device-code, TTL ≤ 10 min, single-use, 4-D
binding, `ZeroizeOnDrop`) stays in-memory. The api already resolves the OAuth
callback `state` against an **in-process** `oauth_state_tokens:
Arc<RwLock<HashMap>>` (`api/src/state.rs`), so OAuth connect is **already
single-replica-only today**. Record this as a **binding operational constraint**
("OAuth connect requires a single replica or sticky callback affinity until
1.1") and ship a typed `PendingExpired` / `PendingNotFound` error + tracing span
so a mid-flow restart surfaces an actionable "restart the connection flow"
instead of a generic 404. Durable multi-replica pending is deferred to 1.1
(alongside ADR-0084 proactive refresh).

## `InMemoryStore` — keep as a permanent `test-util` fixture (do NOT delete)

> **Superseded (2026-06-10):** a follow-up cleanup deleted the `InMemoryStore`
> type outright and moved every test onto a unique in-memory
> `SqliteCredentialStore` (`connect_memory()`), including the moat tests below —
> the SQLite shared-cache backend handles the thundering-herd coalescing race
> correctly (the single-flight lives in the resolver, not the store). The
> reasoning in this section was the conservative call at lock time; eliminating
> the second in-memory CAS reimplementation removed a whole drift-risk class and
> won out. `InMemoryPendingStore` (pending-state) is unaffected and stays.

The 4 engine moat tests (`credential_thundering_herd_tests`,
`refresh_coordinator_*`, `credential_resolve_snapshot_tests`,
`credential_resolver_refresh_coalesced`) **hard-reference the concrete
`InMemoryStore`** directly (not via a composition root), and are
timing/concurrency-sensitive. Plain `sqlite::memory:` is **not** behaviorally
drop-in: each connection is a separate DB unless `mode=memory&cache=shared`, and
`max_connections=1` serializes the very race these tests create. So:

- `InMemoryStore` stays as a `#[cfg(any(test, feature = "test-util"))]` fixture
  (synchronous multi-handle shared state) — it is the conformance "InMemory"
  backend and the moat tests' backend. The "delete the in-memory store" goal is
  realized as **"production composition roots no longer wire it"**, not "the type
  is removed from the test tree."
- Any SQLite-backed concurrency test uses `sqlite:file:<name>?mode=memory&cache=shared`
  with `max_connections > 1`, never plain `:memory:`.

## Phased execution (each phase workspace-green, independently mergeable)

| Phase | Scope | Gate |
|---|---|---|
| **P0 — docs** | This plan + ADR-0088 amendment (A-plus columns, pending constraint, timestamp/CAS rules, `InMemoryStore`-stays-fixture). | docs only |
| **P1 — contract + SQLite store** | Add `StoredCredential.name: Option<String>` (+ all construction sites). `SqliteCredentialStore` impl of `CredentialStore` over `SqlitePool` (refresh_claim pattern; owner_id/name extraction; CAS UPSERT). Migration `0030` (sqlite, incl. DROP of B's tables). Shared conformance suite run against **both** `InMemoryStore` and `SqliteCredentialStore(:memory:,cache=shared)`. | **API-GATE** (DTO field); storage clippy+nextest |
| **P2 — Postgres store** | `PgCredentialStore` (TIMESTAMPTZ/BYTEA/BIGINT, real tx, `ON CONFLICT ... WHERE version`). Migration `0030` (postgres). `DATABASE_URL`-gated tests + skip-clean. | storage clippy+nextest; pg gated |
| **P3 — delete Model B** | Delete `repos/credential.rs`, `rows/credential.rs` (+ `mod.rs` re-exports, `record.rs` cross-ref). Update ADR-0088 §Pain-3 note → "removed". | storage clippy+nextest; doc-link sweep (all forms) |
| **P4 — composition roots + test_support** | Wire `Encryption∘Audit∘Cache∘SqliteCredentialStore` at server/api roots. Repoint `in_memory_service()`/`test_support` to `SqliteCredentialStore(:memory:,cache=shared)`. Typed pending error + span; record the single-replica OAuth constraint. | full workspace clippy+nextest |
| **P5 — stop wiring InMemoryStore in prod** | Production composition roots no longer construct `InMemoryStore`; retire/repurpose the `credential-in-memory` production feature. `InMemoryStore` remains a `test-util` fixture (see above). | full workspace clippy+nextest |

The lifecycle moat consumes the `CredentialStore` trait and is **not** modified in
any phase; only the backend behind the trait changes, and the moat tests keep
their in-memory backend.

## Risks

1. **Silent moat regression** (CAS/version drift InMemory↔SQLite) → shared
   trait-test harness against both impls incl. CAS-on-missing + `reauth_required`
   short-circuit; engine moat tests must pass unchanged.
2. **`sqlite::memory:` concurrency mistrap** → mandatory `mode=memory&cache=shared`
   + `max_connections>1` for any concurrent SQLite test; moat tests stay on
   `InMemoryStore`.
3. **Timestamp ms-truncation divergence** → normalize to millis on all three
   impls; conformance compares `updated_at` only post-round-trip.
4. **`u64→i64` version cast** → explicit guard, no silent `as`.
5. **Pending-ephemeral surprises multi-replica OAuth** → ADR records the
   single-replica/sticky-affinity constraint + typed error; durable pending = 1.1.
