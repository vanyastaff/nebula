---
name: credential refresh coordination — multi-replica safety
status: proposal
date: 2026-04-24
authors: Claude (synthesized from tech-lead + security-lead review; promoted from 37-finding user audit #17)
scope: nebula-storage, nebula-engine, nebula-core
supersedes: []
related:
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - docs/PRODUCT_CANON.md#132-rotation-refresh-seam
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0008-execution-control-queue-consumer.md
  - docs/adr/0017-control-queue-reclaim-policy.md
  - docs/research/n8n-credential-pain-points.md
  - docs/superpowers/archive/2026-04-24-credential-redesign-exploratory/STATUS.md (finding #17 promoted here)
planned-adrs:
  - ADR-0034 — durable credential refresh claim repository
linear: []
---

# Credential refresh coordination — multi-replica safety

## §0 — Context

`nebula-engine::credential::refresh` owns an in-process `RefreshCoordinator` (LRU `parking_lot::Mutex` keyed by `credential_id`) per [ADR-0030](../../adr/0030-engine-owns-credential-orchestration.md) §3 amendment. It coalesces concurrent refresh attempts **within a single replica**.

It does **not** coordinate across replicas. Concrete scenario:

```
Replica A: resolve(slack_cred) — near expiry
Replica A: L1.lock(slack_cred)
Replica A: refresh_coordinator.refresh()
Replica A:   → POST {idp.token_endpoint} with refresh_token_v1
Replica A:   ← response: access_token_v2 + refresh_token_v2 (old invalidated)
Replica A: storage::put(new_state)
Replica A: L1.unlock

Replica B (concurrent):
Replica B: resolve(slack_cred) — same near-expiry moment
Replica B: L1.lock(slack_cred)          ← DIFFERENT MUTEX (different process)
Replica B: read state — but maybe still old (cache lag / race)
Replica B: refresh_coordinator.refresh()
Replica B:   → POST with refresh_token_v1 (stale!)
Replica B:   ← IdP rejects: "refresh token already consumed"
Replica B: credential marked ReauthRequired — permanent failure until user manually reauth
```

This is the n8n #13088 class (confirmed production issue в n8n, unresolved since 2024). Multi-replica Nebula будет hit this under any IdP с refresh_token rotation (Microsoft Azure AD, Google, Slack, most modern OAuth2 providers).

## §1 — Goal

Prevent concurrent refresh of the same credential **across replicas**, while preserving the in-process coalescing that already works.

**Non-goals:**

- Cross-region refresh coordination (future if deployment model changes)
- Distributed rotation scheduler leader election (separate concern — see §9 follow-ups)
- Refresh inside DYNAMIC credential execution-scoped store (different lifecycle)

## §2 — Definition of done

1. Two replicas triggering refresh for the same `credential_id` within a 60-second window result in **one** IdP POST, both observing the fresh state after.
2. Crash of Replica A mid-refresh (claim held, IdP responded, response not persisted) produces `CredentialError::ReauthRequired` visible in `CredentialStatus` within 2× reclaim TTL (default 60s), **not** a silent stuck credential.
3. Noisy-neighbor latency (IdP response delay 20-40s) does not trigger false `ReauthRequired` — sentinel requires N=3 confirmed failures before flagging (security-lead Q1).
4. CI has loom test asserting no two concurrent refreshes can enter the critical section for same `credential_id` across simulated multi-replica scenario.
5. CI has property test asserting `heartbeat < ttl / 3` and `refresh_timeout < ttl - 2 × heartbeat` invariants hold.
6. Migrations for SQLite + Postgres add `credential_refresh_claims` table with schema parity (dialect translation of `FOR UPDATE SKIP LOCKED` semantics).
7. `nebula-credential` MATURITY `Engine integration` column flips `partial → stable` after this spec ships.
8. `docs/OBSERVABILITY.md` entry added для new metrics (`nebula_credential_refresh_coord_*`).

## §3 — Design

### 3.1 Two-tier coordination

```
┌─────────────────────────────────────────────────────────────┐
│                  engine::RefreshCoordinator                 │
│                                                             │
│  ┌─── L1: in-process mutex ─────────────────┐              │
│  │  LruCache<CredentialId, Arc<Mutex<()>>>   │              │
│  │  - coalesces concurrent refresh в этом    │              │
│  │    replica                                │              │
│  │  - fast path: if another coroutine уже    │              │
│  │    refreshing, await release              │              │
│  └───────────────────────────────────────────┘              │
│                       │                                     │
│                       ▼ after L1 acquired                   │
│  ┌─── L2: durable claim ─────────────────────┐              │
│  │  storage::RefreshClaimRepo                │              │
│  │  - CAS: INSERT/UPDATE с WHERE claim_expires│              │
│  │    < NOW() OR holder IS NULL              │              │
│  │  - TTL 30s, heartbeat 10s                 │              │
│  │  - cross-replica coordination             │              │
│  └───────────────────────────────────────────┘              │
└─────────────────────────────────────────────────────────────┘
```

**Acquisition sequence:**

```rust
impl RefreshCoordinator {
    pub async fn refresh_coalesced<F, Fut>(
        &self,
        credential_id: CredentialId,
        do_refresh: F,
    ) -> Result<(), RefreshError>
    where
        F: FnOnce(RefreshClaim) -> Fut,
        Fut: Future<Output = Result<(), RefreshError>>,
    {
        // L1: in-process coalescing
        let l1_mutex = self
            .l1_cache
            .get_or_insert(credential_id, || Arc::new(Mutex::new(())));
        let _l1_guard = l1_mutex.lock().await;

        // Re-check state after L1 acquisition — another coroutine may have refreshed.
        if !self.needs_refresh(&credential_id).await? {
            return Ok(()); // coalesced within replica
        }

        // L2: durable claim with retry on contention
        let claim = self.try_acquire_l2_with_backoff(&credential_id).await?;

        // Start heartbeat in background
        let hb_task = tokio::spawn(heartbeat_loop(self.repo.clone(), claim.token.clone()));

        // Execute refresh under both locks
        let result = do_refresh(claim.clone()).await;

        // Release: stop heartbeat + mark claim released.
        hb_task.abort();
        self.repo.release(claim.token).await?;

        result
    }
}
```

### 3.2 RefreshClaimRepo trait

Living в `nebula-storage/src/credential/refresh_claim.rs`.

```rust
#[async_trait]
pub trait RefreshClaimRepo: Send + Sync + 'static {
    /// Attempts to claim refresh for credential. CAS-based.
    ///
    /// Returns Some(token) if claim acquired. None if another holder still valid.
    ///
    /// Implementations MUST:
    /// - Atomically check claim_expires < NOW() AND CAS insert/update
    /// - Record holder = replica_id for diagnostics
    /// - Return existing claim_expires_at в ClaimConflict for backoff timing
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError>;

    /// Extend claim TTL. Must validate holder matches.
    /// Returns Err(ClaimLost) if our claim expired and another replica took it.
    async fn heartbeat(&self, token: &ClaimToken) -> Result<(), HeartbeatError>;

    /// Release claim. Idempotent.
    async fn release(&self, token: ClaimToken) -> Result<(), RepoError>;

    /// Sweep claims where holder crashed (past TTL).
    /// Returns credential_ids that had expired claims reclaimed.
    /// Typically called on cadence (30s) parallel to control queue reclaim.
    async fn reclaim_stuck(&self) -> Result<Vec<CredentialId>, RepoError>;
}

pub enum ClaimAttempt {
    Acquired(RefreshClaim),
    Contended { existing_expires_at: DateTime<Utc> },
}

pub struct RefreshClaim {
    pub credential_id: CredentialId,
    pub token: ClaimToken,       // opaque handle (UUID + generation)
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct ClaimToken {
    pub claim_id: Uuid,
    pub generation: u64,         // bumped on each CAS — prevents stale heartbeats
}

pub enum HeartbeatError {
    ClaimLost,                   // our claim expired, another holder took it
    Repo(RepoError),
}
```

### 3.3 Schema — `credential_refresh_claims` table

**SQLite:**

```sql
CREATE TABLE credential_refresh_claims (
    credential_id     TEXT    NOT NULL PRIMARY KEY,
    claim_id          TEXT    NOT NULL,          -- UUID
    generation        INTEGER NOT NULL,           -- bumped on each CAS
    holder_replica_id TEXT    NOT NULL,
    acquired_at       TEXT    NOT NULL,           -- ISO-8601
    expires_at        TEXT    NOT NULL,
    sentinel          INTEGER NOT NULL DEFAULT 0  -- 0=normal, 1=refresh_in_flight
);

CREATE INDEX idx_refresh_claims_expires ON credential_refresh_claims(expires_at);
```

**Postgres:**

```sql
CREATE TABLE credential_refresh_claims (
    credential_id     TEXT NOT NULL PRIMARY KEY,
    claim_id          UUID NOT NULL,
    generation        BIGINT NOT NULL,
    holder_replica_id TEXT NOT NULL,
    acquired_at       TIMESTAMPTZ NOT NULL,
    expires_at        TIMESTAMPTZ NOT NULL,
    sentinel          SMALLINT NOT NULL DEFAULT 0
);

CREATE INDEX idx_refresh_claims_expires ON credential_refresh_claims(expires_at);
```

Reclaim sweep query (Postgres example):

```sql
UPDATE credential_refresh_claims
SET claim_id = $new_claim_id,
    generation = generation + 1,
    holder_replica_id = $new_holder,
    acquired_at = NOW(),
    expires_at = NOW() + INTERVAL '30 seconds'
WHERE expires_at < NOW()
RETURNING credential_id;
```

### 3.4 Mid-refresh crash sentinel (security-lead N17 mitigation)

Claim holder marks the claim row `sentinel = 1` immediately before starting IdP POST:

```
1. Acquire claim → sentinel = 0
2. SET sentinel = 1 (mark "refresh_in_flight")
3. POST {idp.token_endpoint}
4. Response received
5. Storage::put(new_state)
6. SET sentinel = 0 (clear sentinel) AND release claim
```

On reclaim sweep, if sweep finds `sentinel = 1` AND `expires_at < NOW()`:

- Record a **sentinel_event** row (NEW table `credential_sentinel_events`):
  ```sql
  INSERT INTO credential_sentinel_events
  (credential_id, detected_at, crashed_holder, generation)
  VALUES ($1, NOW(), $2, $3);
  ```
- Track count of sentinel events per credential per rolling 1-hour window
- **If count ≥ 3 within 1 hour** → mark credential as `ReauthRequired`:
  - Update credential state с status `ReauthRequired`
  - Emit `CredentialEvent::ReauthRequired { credential_id, reason: SentinelRepeated }`
  - Surface to UI via WebSocket
- **If count < 3** → reclaim normally (treat as recoverable)

**Rationale (security-lead Q1):** noisy-neighbor latency + claim timeout can false-trip single sentinel event. Requiring N=3 confirmed events within 1h before permanent `ReauthRequired` prevents DoS from intermittent IdP slowness. Parameters configurable per deployment.

### 3.5 Parameter discipline (finding #16)

```rust
pub struct RefreshCoordConfig {
    /// Claim TTL. Default 30s.
    pub claim_ttl: Duration,
    /// Heartbeat interval. MUST be < claim_ttl / 3.
    pub heartbeat_interval: Duration,
    /// Refresh timeout. MUST be < claim_ttl - 2 × heartbeat_interval.
    pub refresh_timeout: Duration,
    /// Reclaim sweep cadence. MUST be ≤ claim_ttl.
    pub reclaim_sweep_interval: Duration,
    /// Sentinel events threshold for ReauthRequired. Default 3 within 1h.
    pub sentinel_threshold: u32,
    pub sentinel_window: Duration,
}

impl RefreshCoordConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.heartbeat_interval * 3 >= self.claim_ttl {
            return Err(ConfigError::HeartbeatTooSlow);
        }
        if self.refresh_timeout + self.heartbeat_interval * 2 >= self.claim_ttl {
            return Err(ConfigError::RefreshTimeoutTooLong);
        }
        if self.reclaim_sweep_interval > self.claim_ttl {
            return Err(ConfigError::ReclaimTooSlow);
        }
        Ok(())
    }
}
```

Default shape:

```rust
impl Default for RefreshCoordConfig {
    fn default() -> Self {
        Self {
            claim_ttl: Duration::from_secs(30),
            heartbeat_interval: Duration::from_secs(10),
            refresh_timeout: Duration::from_secs(8),
            reclaim_sweep_interval: Duration::from_secs(30),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_secs(3600),
        }
    }
}
```

CI test runs `RefreshCoordConfig::default().validate().is_ok()` asserting shipped defaults are consistent.

### 3.6 Contention backoff

When `try_claim` returns `Contended`, coordinator uses the returned `existing_expires_at` to sleep until that moment (plus small jitter), then re-check state:

```rust
async fn try_acquire_l2_with_backoff(&self, id: &CredentialId) -> Result<RefreshClaim, RefreshError> {
    let max_attempts = 5;
    for attempt in 0..max_attempts {
        match self.repo.try_claim(id, &self.replica_id, self.config.claim_ttl).await? {
            ClaimAttempt::Acquired(c) => return Ok(c),
            ClaimAttempt::Contended { existing_expires_at } => {
                // After contender's claim expires OR they finish + release, re-check.
                let wait_until = existing_expires_at.min(Utc::now() + Duration::from_secs(5));
                let delay = (wait_until - Utc::now()).to_std().unwrap_or(Duration::from_millis(200));
                tokio::time::sleep(delay + jitter_ms(100)).await;

                // CRITICAL: re-check state — their refresh may have succeeded.
                if !self.needs_refresh(id).await? {
                    return Err(RefreshError::CoalescedByOtherReplica);
                }
            }
        }
    }
    Err(RefreshError::ContentionExhausted)
}
```

`CoalescedByOtherReplica` is **not** an error — the caller treats it as success (credential freshly refreshed by another replica).

## §4 — Canon adherence

- **§12.5 secrets-and-auth:** new claim and sentinel rows carry **no** credential material. Only `credential_id`, `holder_replica_id`, timestamps, UUIDs, and sentinel boolean. Audit surface limited by construction.
- **§13.2 rotation-refresh seam:** `Credential::refresh()` trait method unchanged. This spec wraps the refresh call inside the coordinator's two-tier lock; does not change what happens **inside** refresh.
- **§14 no discard-and-log:** sentinel events, when threshold exceeded, **block** the credential (not just log). `ReauthRequired` state surfaces to operator and user.
- **§4.5 operational honesty:** `nebula-credential` MATURITY flips `Engine integration partial → stable` **only after** this spec ships end-to-end. Until then, row stays `partial`.

## §5 — Testing

### 5.1 Loom tests

```rust
#[test]
fn no_concurrent_refresh_across_replicas() {
    loom::model(|| {
        let repo = Arc::new(LoomRefreshClaimRepo::new());
        let cred_id = CredentialId::new();
        let refreshed_count = Arc::new(AtomicU32::new(0));

        let h1 = loom::thread::spawn({
            let repo = repo.clone();
            let cnt = refreshed_count.clone();
            move || {
                let coord = RefreshCoordinator::new(repo, ReplicaId::new("A"));
                coord.refresh_coalesced_sync(cred_id, || {
                    cnt.fetch_add(1, Ordering::Relaxed);
                });
            }
        });

        let h2 = loom::thread::spawn({
            let repo = repo.clone();
            let cnt = refreshed_count.clone();
            move || {
                let coord = RefreshCoordinator::new(repo, ReplicaId::new("B"));
                coord.refresh_coalesced_sync(cred_id, || {
                    cnt.fetch_add(1, Ordering::Relaxed);
                });
            }
        });

        h1.join().unwrap();
        h2.join().unwrap();

        // Only ONE refresh should have occurred.
        assert_eq!(refreshed_count.load(Ordering::Relaxed), 1);
    });
}
```

### 5.2 Property tests

```rust
#[test]
fn refresh_coord_config_always_valid_with_defaults() {
    let cfg = RefreshCoordConfig::default();
    prop_assert!(cfg.validate().is_ok());
}

proptest! {
    #[test]
    fn heartbeat_always_less_than_ttl_third(
        ttl_secs in 5u64..300,
        hb_secs in 1u64..100,
    ) {
        let cfg = RefreshCoordConfig {
            claim_ttl: Duration::from_secs(ttl_secs),
            heartbeat_interval: Duration::from_secs(hb_secs),
            refresh_timeout: Duration::from_secs(1),
            reclaim_sweep_interval: Duration::from_secs(1),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_secs(3600),
        };

        let valid = cfg.validate().is_ok();
        let invariant_holds = hb_secs * 3 < ttl_secs;

        prop_assert_eq!(valid, invariant_holds);
    }
}
```

### 5.3 Integration tests (Postgres)

```rust
#[tokio::test]
async fn mid_refresh_crash_triggers_sentinel() {
    let pool = test_postgres().await;
    let repo = PgRefreshClaimRepo::new(pool.clone());
    let cred_id = CredentialId::new();

    // Replica A acquires claim + marks sentinel + "crashes" (drops coordinator before release)
    {
        let coord_a = RefreshCoordinator::new(Arc::new(repo.clone()), ReplicaId::new("A"));
        let claim = coord_a.acquire_for_test(&cred_id).await.unwrap();
        repo.mark_sentinel(&claim.token).await.unwrap();
        // drop coord_a без release — simulates crash
    }

    // Wait for claim TTL to expire.
    tokio::time::sleep(Duration::from_secs(35)).await;

    // Replica B runs reclaim sweep. Should detect sentinel + record event.
    let coord_b = RefreshCoordinator::new(Arc::new(repo.clone()), ReplicaId::new("B"));
    let reclaimed = repo.reclaim_stuck().await.unwrap();
    assert_eq!(reclaimed, vec![cred_id]);

    let events = repo.sentinel_events_for(&cred_id).await.unwrap();
    assert_eq!(events.len(), 1);
}

#[tokio::test]
async fn sentinel_below_threshold_does_not_flag_reauth() {
    // 2 crashes within 1 hour — below threshold 3 — credential stays usable.
    // (long-running; marked slow in CI)
}

#[tokio::test]
async fn sentinel_at_threshold_flags_reauth() {
    // 3 crashes within 1 hour → ReauthRequired.
}
```

### 5.4 Chaos test (nightly)

Spin up 3 in-memory replicas sharing a single Postgres. Drive 1000 concurrent resolves against 100 credentials с 30% near-expiry rate over 10 minutes. Assert:

- Each credential refresh invoked at most once per expiry window (no duplicate IdP hits)
- No sentinel false-positives (ReauthRequired count = 0 assuming no injected crash)
- P99 resolve latency < 100ms outside refresh window

## §6 — Observability

New metrics in `nebula-metrics::naming`:

```rust
pub const NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL: &str =
    "nebula_credential_refresh_coord_claims_total";    // labels: outcome={acquired,contended,exhausted}

pub const NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL: &str =
    "nebula_credential_refresh_coord_coalesced_total"; // labels: tier={l1,l2}

pub const NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL: &str =
    "nebula_credential_refresh_coord_sentinel_events_total"; // labels: action={recorded,reauth_triggered}

pub const NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL: &str =
    "nebula_credential_refresh_coord_reclaim_sweeps_total"; // labels: outcome={reclaimed,no_work}

pub const NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS: &str =
    "nebula_credential_refresh_coord_hold_duration_seconds"; // histogram
```

Tracing spans:

- `credential.refresh.coordinate(credential_id, replica_id, tier)` wrapping full refresh_coalesced call
- `credential.refresh.claim.acquire(credential_id, attempt)` per L2 claim attempt
- `credential.refresh.sentinel.detected(credential_id, crashed_holder, recent_count)` when sweep detects sentinel

Audit events (through existing `AuditLayer`):

- `RefreshCoordClaimAcquired { credential_id, holder, ttl }`
- `RefreshCoordSentinelTriggered { credential_id, recent_count }`
- `RefreshCoordReauthFlagged { credential_id, reason: SentinelRepeated }`

## §7 — Migration

### 7.1 Schema migration

1. Storage crate adds migration:
   - `crates/storage/migrations/sqlite/NNNN_credential_refresh_claims.sql`
   - `crates/storage/migrations/postgres/NNNN_credential_refresh_claims.sql`
2. Companion sentinel events table (same migration).
3. CI job `schema-parity` asserts both dialects evolve together (existing check extended).

### 7.2 Code migration

1. New module `crates/storage/src/credential/refresh_claim.rs` с trait + `InMemoryRefreshClaimRepo` impl.
2. New module `crates/storage/src/credential/refresh_claim/pg.rs` с `PgRefreshClaimRepo` impl.
3. Refactor `crates/engine/src/credential/refresh.rs`:
   - Rename current `RefreshCoordinator` → `L1RefreshCoalescer` (internal, private)
   - New `RefreshCoordinator` struct wraps L1 + L2 claim repo
   - Public surface preserved: callers still use `refresh_coalesced(credential_id, do_refresh)`
4. `AppState` / engine composition root accepts `Arc<dyn RefreshClaimRepo>` during construction. Default impl для tests = `InMemoryRefreshClaimRepo`. Production composition injects `PgRefreshClaimRepo` or `SqliteRefreshClaimRepo`.
5. `token_refresh.rs` wraps its HTTP POST inside sentinel set/clear sequence.
6. Reclaim sweep task spawned in `engine::init()` alongside existing control-queue reclaim (parallel task).

### 7.3 Desktop mode (SQLite single-replica)

Single-replica deployment: `PgRefreshClaimRepo` replaced with `SqliteRefreshClaimRepo`; all CAS semantics translate via `UPDATE ... WHERE claim_id = ?` (no `FOR UPDATE SKIP LOCKED` needed since no concurrent writers from different processes in desktop mode). Sentinel detection still works (single replica can still crash mid-refresh).

### 7.4 Rollout phases

**P1 — storage infra (1 PR):**

- RefreshClaimRepo trait + DTOs в `nebula-storage/src/credential/refresh_claim.rs`
- InMemoryRefreshClaimRepo impl + unit tests
- SQLite migration + SqliteRefreshClaimRepo
- Postgres migration + PgRefreshClaimRepo
- Loom test for atomic CAS semantics
- CI schema parity check passes

**P2 — engine coordinator refactor (1 PR, depends P1):**

- Rename existing coordinator to L1RefreshCoalescer
- Introduce new RefreshCoordinator wrapping L1+L2
- Plumb Arc<dyn RefreshClaimRepo> through composition root
- Update `token_refresh.rs` to mark sentinel before IdP POST + clear after
- Reclaim sweep background task
- Property tests on config invariants
- Integration test: mid-refresh crash → sentinel recorded

**P3 — sentinel threshold logic (1 PR, depends P2):**

- Sentinel events table schema + migration (extend P1's migration if P2 didn't ship)
- ReauthRequired credential state transition
- CredentialEvent::ReauthRequired publish
- Threshold config parameters + validation
- Integration tests для threshold behavior (below / at / above)

**P4 — observability (1 PR, depends P3):**

- Metrics constants в nebula-metrics::naming
- Tracing spans in coordinator + reclaim
- Audit events through AuditLayer
- `docs/OBSERVABILITY.md` entry
- Dashboard examples (optional deliverable)

**P5 — MATURITY flip (1 PR, depends P4):**

- Update `docs/MATURITY.md` credential row `Engine integration: partial → stable`
- `CHANGELOG.md` entry
- Update `docs/adr/0030-engine-owns-credential-orchestration.md` §3 amendment date/status note

Total estimate: **5 PRs, ~1.5 weeks of focused work** by one engineer with ~2 days review per PR.

## §8 — Risks & trade-offs

### 8.1 Accepted risks

1. **Extra storage call per refresh.** L2 claim adds ~1-5ms Postgres round trip per refresh. Acceptable — refresh is seconds-scale operation, claim overhead negligible.
2. **Claim table grows unbounded if reclaim sweep fails.** Mitigated by: (a) sweep cadence 30s; (b) reclaim deletes rather than accumulates; (c) CI chaos test includes sweep-failing scenario.
3. **Sentinel threshold is a heuristic.** N=3 within 1h может be wrong for specific deployments. Configurable per replica config. Document tuning guidance in runbook.

### 8.2 Rejected alternatives

**(a) External coordinator (etcd/Zookeeper/Consul).**
Rejected: Nebula local-first (canon §12.3), external dependency violates desktop/self-hosted default.

**(b) Postgres advisory locks (`pg_advisory_lock`).**
Rejected: Postgres-only. No SQLite equivalent. Would create asymmetric behavior between desktop and production modes.

**(c) Just accept the race (document as known limitation).**
Rejected: n8n #13088 is real customer pain, unresolved in n8n for 2+ years. Nebula positioning (canon §4.5 operational honesty) требует preventing this at architecture level, not folklore.

**(d) Single-writer replica election (only one replica refreshes all credentials).**
Rejected: hot-spot на leader replica; leader failure stalls все refreshes; adds another dimension of distributed coordination complexity. Two-tier per-credential approach more resilient.

## §9 — Follow-ups

- **ADR-0034 — durable credential refresh claim repository** — ADR citing this spec. Lands с P1 PR.
- **Distributed rotation scheduler leader election** — separate concern. Current rotation scheduler (ADR-0030 §2) is also single-replica today. When multi-replica rotation becomes needed, leader claim repo pattern generalizes from here.
- **Cross-region refresh coordination** — if Nebula deployment model evolves к multi-region, cross-region claim sharing (regional Postgres replication lag considerations). Out of scope today.
- **Refresh backoff на provider errors** — if IdP returns 429 / temporary errors, current design retries on next `resolve` call. Future enhancement: circuit breaker per provider, visible via `CredentialStatus::Degraded`.
- **RefreshCoordinator trait in nebula-core evolution** — existing core trait `acquire_refresh(&str) -> RefreshToken` designed for single-tier. This spec's two-tier `RefreshCoordinator` is a concrete struct, not a trait. If future sharing с other crates needed, consider evolving core's trait shape.

## §10 — Test coverage requirements for DoD

- [ ] `loom` test demonstrating atomic CAS under 2-thread interleaving
- [ ] Property test on `RefreshCoordConfig::validate()` invariants
- [ ] Integration test: sequential 2-replica refresh → only 1 IdP call
- [ ] Integration test: mid-refresh crash → sentinel event recorded
- [ ] Integration test: sentinel count < threshold → no ReauthRequired
- [ ] Integration test: sentinel count ≥ threshold → ReauthRequired flagged
- [ ] Integration test: `CoalescedByOtherReplica` handled gracefully (not error)
- [ ] Integration test: claim heartbeat extends TTL; missed heartbeat → ClaimLost
- [ ] Chaos test (nightly): 3 replicas × 100 credentials × 10 min — zero false-positives
- [ ] Schema parity CI passes для new migrations
- [ ] `cargo deny` rules unchanged (no new deps added к `nebula-credential`)

Тесты landed в PRs P1-P3.

## §11 — Open questions

1. **Should `RefreshCoordinator` be re-entrant if same replica tries to refresh same credential twice?** Currently L1 mutex ensures it waits. L2 claim generation prevents double heartbeat. This should be documented as "yes, re-entry waits cleanly."

2. **What if ProviderRegistry (future) имеет multiple `token_endpoint` variants per provider (Microsoft multi-tenant)?** Should claim key include `(credential_id, endpoint_hash)` or just `credential_id`? Current proposal: just `credential_id` (endpoint is fixed per credential instance). If endpoint can change mid-lifetime, that's config rotation — separate spec.

3. **Behavior when Postgres `credential_refresh_claims` table unavailable (DB degraded)?** Fall back to L1-only (in-process coalesce)? Or fail-closed refresh (no refreshes until storage recovers)? Proposal: L1-only fallback with loud WARN metric + audit event. Operator sees "degraded coordination" and acts.

---

**End of Spec H0.**

Next step: write ADR-0034 citing this spec, land P1 migration + RefreshClaimRepo trait.
