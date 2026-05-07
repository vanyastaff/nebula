---
id: 0048
title: idempotency-store-backend
status: accepted
date: 2026-05-07
supersedes: []
superseded_by: []
tags: [api, idempotency, storage, m3]
related:
  - crates/api/src/middleware/idempotency.rs
  - crates/storage/src/repos/control_queue.rs
  - .ai-factory/ROADMAP.md
  - .ai-factory/plans/m3-4-idempotency.md
---

# 0048. Idempotency-Store Backend — Hybrid (in-memory + PG-backed)

## Context

`nebula-api` ships `IdempotencyLayer` + `IdempotencyStore` trait +
`InMemoryIdempotencyStore` (PR #638). The middleware caches `2xx` / `4xx`
responses keyed by
`(method, path, Idempotency-Key, identity-fingerprint, body-fingerprint)` and
replays them within a 24h TTL window. The in-memory backend is correct in
isolation, but the layer is **not yet mounted in
`crate::app::build_app`** — the explicit TODO at
`crates/api/src/middleware/idempotency.rs:48-50` records this gap. Production
POST endpoints lack replay protection today.

ROADMAP §M3.4 exit criteria require:

- mount the layer in `build_app`;
- a shared-store decision (in-memory vs durable) ratified in an ADR;
- end-to-end integration test against the real `build_app` router (not
  minimal test routers);
- `nebula_api_idempotency_*` metrics + tracing span fields.

The §M3 1.0 closure criteria block (`.ai-factory/ROADMAP.md` §M3 _1.0
closure criteria_) goes further: **"the dedup store survives process
restart in production deployments"**. A pure in-memory backend cannot
satisfy this — multi-runner deployments lose dedup state on restart and
cannot share state across processes without external coordination. That is
the load-bearing constraint this ADR resolves.

The Nebula workspace already runs PostgreSQL as the primary durable
backend — `nebula-storage` ships 23 PG migrations, a sqlx pool wired
through every long-lived service, and the `ControlQueueRepo` precedent
for `Repo` traits with `InMemory<X>Repo` + `Pg<X>Repo` impls plus
`DATABASE_URL`-gated PG tests behind the `postgres` feature
(`crates/storage/src/repos/control_queue.rs`). Adding a new durable
backend reuses existing plumbing; introducing a different store family
(Redis, Memcached, etcd) would widen blast radius for one feature.

## Decision

**Adopt a hybrid backend model:** keep `InMemoryIdempotencyStore` for
dev / single-process / tests; introduce an `IdempotencyStoreRepo` trait in
`nebula-storage` (Exec layer) and a `PgIdempotencyStore` impl that the
`-api` crate bridges into the existing `IdempotencyStore` middleware
contract. The composition root selects the backend from
`ApiConfig.idempotency.backend` (`memory` | `postgres`).

### Backend selection contract

- **Default in `for_test()` and `from_env` defaults:** `Memory`.
- **Production deployments:** operators set
  `API_IDEMPOTENCY_BACKEND=postgres` and rely on the same
  `DATABASE_URL` already used by storage / engine.
- **Fail-closed.** Selecting `Postgres` without a configured
  `DATABASE_URL` is a hard startup error in the composition root —
  the API binary refuses to boot, mirroring the
  `JwtSecret::DEV_PLACEHOLDER` policy. No silent fallback to
  in-memory (per `feedback_no_shims.md`).
- **Operator warning.** Selecting `Memory` outside dev mode emits a
  `tracing::warn!` at startup with the explicit
  "dedup state is lost on restart and across runners" message so the
  failure mode is visible in operational logs.

### Trait shape (Exec-layer contract, no `http` types)

```rust
// crates/storage/src/repos/idempotency.rs
#[async_trait]
pub trait IdempotencyStoreRepo: Send + Sync + std::fmt::Debug {
    async fn get(&self, cache_key: &str) -> Result<Option<CachedRecord>, StorageError>;
    async fn put(
        &self,
        cache_key: String,
        record: CachedRecord,
        ttl: Duration,
    ) -> Result<(), StorageError>;
    async fn evict_expired(&self) -> Result<u64, StorageError>;
}

#[derive(Clone, Debug)]
pub struct CachedRecord {
    pub status: u16,
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: Vec<u8>,
    pub fingerprint: [u8; 32],
}
```

`Vec<(String, Vec<u8>)>` (rather than `http::HeaderMap`) keeps the Exec
layer free of `http` types. The `-api` bridge struct
(`StorageBackedIdempotencyStore<R: IdempotencyStoreRepo>`) reconstructs
`HeaderMap` on read and maps `StorageError` into a typed
`IdempotencyStoreError::Decode { source }` for malformed payloads. A
decode failure surfaces as 500 from the middleware — falling back to a
"cache miss" would silently drop replay protection on data corruption
and is rejected.

### Concurrency contract

`PgIdempotencyStore::put` uses `INSERT ... ON CONFLICT (cache_key) DO
NOTHING` so concurrent first-writer-wins matches the in-memory
semantics (`moka` `entry().or_insert_with`). The middleware does not
retry; a racer's `INSERT` is a no-op and the next `get` from a retried
caller hits the winner's row. `evict_expired` is an out-of-band
maintenance sweep, called from a startup background task whose cadence
is `IdempotencyApiConfig.sweep_interval_secs` (default 300 s; `0`
disables for dev / single-process runs; `< 60` is a `tracing::warn!`
sanity floor to surface obvious misconfigurations without a hard
reject — operators may have legitimate reasons in dev clusters).

### Layer placement (mount position)

`IdempotencyLayer` mounts on **`api_routes` BEFORE the webhook transport
merge**, not on the post-merge router. Webhook ingress has its own
dedup contract (ROADMAP §M3.3 — provider signature + replay-window
timestamp); routing webhook traffic through the API idempotency cache
would inflate `nebula_api_idempotency_misses_total` with provider
traffic that never carries `Idempotency-Key` and conflate two distinct
dedup surfaces.

Position relative to the outer middleware stack:

```text
outermost                                                            innermost
 rate_limit → request_id → security_headers → middleware_stack → idempotency → routes
                                                                  (api only)
```

A cached replay still receives a fresh `X-Request-ID` and security
headers because those layers wrap idempotency.

## Alternatives Rejected

### Alternative 1: In-memory only

Rejected on the §M3 1.0 closure criterion. Multi-runner deployments
cannot share dedup state without external coordination; restart loses
it entirely. PR #638's middleware is correct on a single process — the
gap is durability, not correctness.

### Alternative 2: Redis-backed store

Rejected on first-principles dependency-budget analysis. No existing
Redis dependency in the workspace; adding one for a single feature
widens the blast radius (operational footprint, deploy topology,
license review, security review) for marginal capability gain over PG.
Defer until a second feature pulls Redis in for orthogonal reasons
(e.g. M5 rate-limiting at edge, if it ever supersedes the in-process
governor). When that lands, this ADR can be superseded with an
adapter; until then, PG is the right shape.

### Alternative 3: Skip the durable backend, defer §M3.4 to 1.1

Rejected. §M3.4 is one of the four remaining 1.0 API blockers (alongside
§M3.5 trace-context propagation and §M3.6 shift-left validation —
§M3.1 / §M3.2 / §M3.3 closed). Shipping the layer mounted with only
the in-memory backend would leave a known-incorrect production default
documented as "ship-blocking ROADMAP item" — the worst of both worlds.

### Alternative 4: External coordination service (etcd, Memcached, distributed cache)

Rejected on the same dependency-budget reasoning as Redis. No
incumbent service of these classes exists in the deployment, and the
write rate / TTL pattern (24 h TTL, low-thousands writes/sec headroom)
fits comfortably inside a PG table with an `expires_at` index.

## Consequences

### Positive

- §M3 1.0 closure criterion satisfied: dedup state survives process
  restart and is shared across runners through the canonical PG.
- Reuses the existing `nebula-storage` plumbing (`postgres` feature,
  shared sqlx pool, migration runner, `ControlQueueRepo` precedent for
  `Repo` traits + `InMemory<X>Repo` + `Pg<X>Repo`).
- Composition-root binary remains a thin wiring layer; the choice
  surface (`API_IDEMPOTENCY_BACKEND`) is one env var.
- Observation triple (DoD) is structurally clean: typed
  `StorageError` variants, `tracing::Span` on every `get` / `put`,
  invariant `debug_assert!(record.expires_at > now())` on insert.

### Negative

- One forward migration (`0024_add_idempotency_dedup.sql`) under PG +
  SQLite parity (per ADR-0009). Trivial schema; no data migration.
- One additional `Result<T, StorageError>` boundary in the middleware
  hot path. Typed conversion at the bridge struct keeps this contained.
- Two impls (`InMemory` + `Pg`) to keep in sync. The shared `mod tests`
  contract test (round-trip, body-mismatch race, TTL expiry) protects
  against semantic drift.
- Concurrent first-writer-wins under contention is not loom-checked
  in this milestone; deferred to a property-test ticket under M8.1
  (loom probe). Tracked as an explicit gap, not a silent omission.
- `utoipa-axum` / OpenAPI surface picks up one new operator-facing env
  var triplet (`API_IDEMPOTENCY_BACKEND`,
  `API_IDEMPOTENCY_TTL_SECS`, sweep cadence) — documented in the
  per-crate README under §M3.4 closure (Phase F3).

### Neutral

- Header byte-encoding stays as `Vec<(String, Vec<u8>)>` rather than a
  custom length-prefixed binary blob. Density loss is a few bytes per
  cached entry; debugability gain via `psql` pretty-print is real.
  When / if the tradeoff inverts, the migration is internal to
  `pg/idempotency.rs` only — the trait shape does not change.
- Sweep cadence default 300 s is a heuristic balanced for low-traffic
  clusters; high-traffic operators tune it via env. The `< 60`
  warn-floor surfaces obvious misconfigurations without dictating
  policy.

## Migration

Single forward migration under
`crates/storage/migrations/postgres/0024_add_idempotency_dedup.sql`
(plus SQLite parity per ADR-0009 forward-compat):

```sql
CREATE TABLE IF NOT EXISTS api_idempotency_dedup (
    cache_key   TEXT PRIMARY KEY,
    status      SMALLINT NOT NULL,
    headers     BYTEA NOT NULL,
    body        BYTEA NOT NULL,
    fingerprint BYTEA NOT NULL,
    expires_at  TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS api_idempotency_dedup_expires_at_idx
    ON api_idempotency_dedup (expires_at);
```

No data migration. Rolling deploys on the in-memory backend continue to
work mid-upgrade; flipping `API_IDEMPOTENCY_BACKEND=postgres` after the
migration applies turns on cross-process dedup. Rolling back to
`memory` is a config change, not a schema change — drop-table on the
PG side is a separate operator action if they want to reclaim disk.

## Observation Triple (DoD)

- **Typed errors:**
  `StorageError::Codec { source }` for malformed-headers reads and
  `IdempotencyStoreError::Decode { source }` at the bridge boundary.
  Both bubble up as 500 — never silently degrade to "cache miss".
- **Tracing spans:** every `get` / `put` carries a span with
  `cache_key_prefix` (first 8 chars — never the full key, to avoid
  leaking client identity into logs), `body_size_bytes`, and
  `outcome`. The sweep task logs `rows_evicted` at INFO every cycle,
  errors at WARN.
- **Invariant checks:**
  `debug_assert!(record.expires_at > now())` on insert paths;
  `debug_assert!(config.idempotency.ttl_secs > 0)` at `build_app`
  layer-mount when `idempotency_store.is_some()`.

Metrics surface (defined in `nebula-metrics::naming` under §M3.4
Phase C1):

- `nebula_api_idempotency_hits_total` (counter)
- `nebula_api_idempotency_misses_total` (counter)
- `nebula_api_idempotency_rejects_total` (counter, label
  `reason ∈ {invalid_key, body_mismatch, body_too_large, non_ascii_header}`)
- `nebula_api_idempotency_store_saturation_ppm` (gauge,
  `entries / max_capacity` scaled by 1_000_000 to fit the i64-backed
  `Gauge` primitive without precision loss; mirrors
  `nebula_eventbus_drop_ratio_ppm`)
- `nebula_api_idempotency_latency_ms` (histogram, full middleware
  path latency in milliseconds)

## Open Questions / Follow-ups

- **Loom-checked first-writer-wins property.** Deferred to M8.1 as a
  property-test ticket (loom probe pattern matches
  `crates/storage-loom-probe`). Tracked, not silent.
- **Sweep cadence sanity floor.** `< 60` warn at startup; reconsider
  if operator feedback shows a sharp need for hard rejection.
- **Restart-survival e2e test.** Phase E3 ships it gated on
  `DATABASE_URL`; if the test rig proves heavy on local dev,
  gate it behind `feature = "pg-e2e"`.

## Pointers

- ROADMAP §M3.4: `.ai-factory/ROADMAP.md:335-355`
- Implementation plan: `.ai-factory/plans/m3-4-idempotency.md`
- Middleware (today): `crates/api/src/middleware/idempotency.rs`
- Repo precedent: `crates/storage/src/repos/control_queue.rs`
- App composition root: `crates/api/src/app.rs::build_app`
- AppState: `crates/api/src/state.rs`
- ADR-0009 (PG/SQLite parity): `docs/adr/0009-storage-migrations.md`
  (parent project) — forward-compat applies here verbatim.
- Feedback memories applied: `feedback_observability_as_completion.md`,
  `feedback_no_shims.md`, `feedback_type_enforce_not_discipline.md`,
  `feedback_active_dev_mode.md`.
