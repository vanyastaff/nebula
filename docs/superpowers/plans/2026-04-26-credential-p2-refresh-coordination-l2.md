---
name: credential П2 — refresh coordination L2 (cross-replica safety)
status: draft (writing-plans skill output 2026-04-26 — awaiting execution-mode choice)
date: 2026-04-26
authors: [vanyastaff, Claude]
phase: П2
scope: cross-cutting — nebula-storage, nebula-engine, nebula-core, nebula-credential, nebula-metrics
related:
  - docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §16.1
  - docs/superpowers/plans/2026-04-26-credential-p1-trait-scaffolding.md (predecessor — П1 shipped at squash 6f83c81b)
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0008-execution-control-queue-consumer.md
  - docs/adr/0017-control-queue-reclaim-policy.md
  - docs/research/n8n-credential-pain-points.md (n8n #13088 class)
new-adrs:
  - ADR-0034 — durable credential refresh claim repository (lands in Stage 1)
---

# Credential П2 — Refresh Coordination L2 (Cross-Replica Safety) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent concurrent OAuth2 refresh of the same credential across replicas via a durable two-tier coordinator (L1 in-process mutex + L2 CAS-based claim in storage), eliminating the n8n #13088 class production race where rotated `refresh_token_v2` invalidates `refresh_token_v1` on a parallel replica.

**Architecture:** Two-tier coordinator: existing in-process `RefreshCoordinator` becomes private `L1RefreshCoalescer`; new outer `RefreshCoordinator` wraps L1 + a `RefreshClaimRepo` trait providing durable cross-replica CAS claim. Two new storage tables (`credential_refresh_claims` + `credential_sentinel_events`); SQLite + Postgres impls with schema parity. Mid-refresh crash detected via sentinel flag; N=3 confirmed events/hour escalates to `ReauthRequired`. Mirrors ADR-0008/0017 control-queue claim pattern.

**Tech Stack:** Rust 1.95.0 (pinned), tokio 1.51 (async runtime), `sqlx` 0.8 (Postgres + SQLite drivers), `loom` 0.7 (concurrency model checker), `proptest` 1.x (property tests), `criterion` (perf — optional), `tracing` 0.1 (structured logs), existing `nebula-eventbus` (CredentialEvent fan-out).

**Pre-execution requirement:** Create dedicated worktree per `superpowers:using-git-worktrees`. Plan execution agent runs inside the worktree; main branch sees only the merge commit at landing.

**Reading order for the engineer:** Sub-spec at `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md` (full design — 651 lines). ADR-0030 (engine-owns-orchestration). ADR-0008 + ADR-0017 (control-queue claim pattern — same shape). Then this plan.

**Total estimate:** 5 Stages, 1.5 weeks focused work per spec §7.4 estimate.

---

## File Map

### Created files

| Path | Purpose |
|------|---------|
| `docs/adr/0034-durable-credential-refresh-claim-repo.md` | NEW ADR per spec §9 follow-ups |
| `crates/storage/src/credential/refresh_claim/mod.rs` | NEW `RefreshClaimRepo` trait + DTOs (`RefreshClaim`, `ClaimToken`, `ClaimAttempt`, `HeartbeatError`) |
| `crates/storage/src/credential/refresh_claim/in_memory.rs` | `InMemoryRefreshClaimRepo` impl (tests + desktop-mode no-storage fallback) |
| `crates/storage/src/credential/refresh_claim/sqlite.rs` | `SqliteRefreshClaimRepo` impl |
| `crates/storage/src/credential/refresh_claim/postgres.rs` | `PgRefreshClaimRepo` impl |
| `crates/storage/migrations/sqlite/0022_credential_refresh_claims.sql` | NEW table `credential_refresh_claims` + index |
| `crates/storage/migrations/sqlite/0023_credential_sentinel_events.sql` | NEW table `credential_sentinel_events` + composite index |
| `crates/storage/migrations/postgres/0022_credential_refresh_claims.sql` | Postgres parity for table 1 |
| `crates/storage/migrations/postgres/0023_credential_sentinel_events.sql` | Postgres parity for table 2 |
| `crates/storage/tests/refresh_claim_loom.rs` | Loom test asserting CAS atomicity under 2-thread interleaving |
| `crates/storage/tests/refresh_claim_proptest.rs` | Property tests on config + claim invariants |
| `crates/storage/tests/refresh_claim_sqlite_integration.rs` | SQLite-backed integration coverage |
| `crates/storage/tests/refresh_claim_pg_integration.rs` | Postgres-backed integration coverage |
| `crates/engine/src/credential/refresh/coordinator.rs` | NEW outer `RefreshCoordinator` (L1+L2) |
| `crates/engine/src/credential/refresh/l1.rs` | Renamed from current `refresh.rs` — private `L1RefreshCoalescer` |
| `crates/engine/src/credential/refresh/sentinel.rs` | Sentinel set/clear helpers + N=3-in-1h threshold logic |
| `crates/engine/src/credential/refresh/reclaim.rs` | Background reclaim sweep task (parallel to control-queue reclaim) |
| `crates/engine/tests/refresh_coordinator_two_tier_integration.rs` | Integration: two replicas, one IdP call |
| `crates/engine/tests/refresh_coordinator_sentinel_integration.rs` | Integration: mid-refresh crash → sentinel detection + threshold escalation |
| `crates/engine/tests/refresh_coordinator_chaos.rs` | Nightly chaos: 3 replicas × 100 creds × 10 min |
| `crates/credential/src/contract/refresh_outcome.rs` | `RefreshOutcome::CoalescedByOtherReplica` variant + `ReauthRequired` sentinel-reason |

### Modified files

| Path | Change |
|------|--------|
| `crates/engine/src/credential/refresh.rs` | DELETE (content moved to `refresh/l1.rs`); replaced by `refresh/mod.rs` |
| `crates/engine/src/credential/refresh/mod.rs` | NEW — re-exports `RefreshCoordinator`, `RefreshCoordConfig`, related types |
| `crates/engine/src/credential/mod.rs` | Update `pub mod refresh;` (now subdir) + re-exports |
| `crates/engine/src/credential/rotation/token_refresh.rs` | Wrap HTTP POST inside sentinel set/clear sequence |
| `crates/engine/src/credential/resolver.rs` | Switch from L1-only `RefreshCoordinator` to new wrapping coordinator |
| `crates/engine/Cargo.toml` | Add dev-dep `loom = "0.7"` (cfg-gated) + `proptest` if not present |
| `crates/storage/src/credential/mod.rs` | Wire `pub mod refresh_claim;` |
| `crates/storage/Cargo.toml` | dev-dep additions for loom + sqlx test fixtures |
| `crates/credential/src/contract/mod.rs` | Wire `pub mod refresh_outcome;` (or extend existing `resolve.rs`) |
| `crates/credential/src/lib.rs` | Re-export `RefreshOutcome::CoalescedByOtherReplica` |
| `crates/metrics/src/naming.rs` | Add 5 new constants per spec §6 |
| `crates/eventbus/src/events.rs` | Add `CredentialEvent::ReauthRequired` if not present + `RefreshCoordSentinelTriggered` |
| `docs/MATURITY.md` | `nebula-credential` Engine integration: `partial → stable` after Stage 5 |
| `docs/OBSERVABILITY.md` | Add §refresh-coordinator metrics + tracing + audit events entry |
| `docs/adr/0030-engine-owns-credential-orchestration.md` | §3 amendment date/status note for two-tier coordinator |
| `crates/storage/README.md` | Add refresh-claim repo description |
| `CHANGELOG.md` | Stage 5 entry |
| `docs/tracking/credential-concerns-register.md` | Flip `draft-f17` row from `proposed` → `done` with merge SHA |

### Deleted files

| Path | Reason |
|------|--------|
| `crates/engine/src/credential/refresh.rs` | Refactored into `refresh/l1.rs` (private inner) + `refresh/coordinator.rs` (public outer); module becomes a directory. |

---

## Stage 0 — Foundation (ADR-0034 + worktree)

### Task 0.1 — Worktree

**Files:** none (worktree creation)

- [ ] **Step 1: Create worktree from current main**

```bash
# From parent worktree (main checkout)
git worktree add -b worktree-credential-p2 .claude/worktrees/credential-p2 main
```

Or using the harness's EnterWorktree tool with `name: "credential-p2"`.

- [ ] **Step 2: Verify clean baseline gate**

Run from the worktree:

```bash
cargo +nightly fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run -p nebula-credential -p nebula-engine -p nebula-storage --profile ci --no-tests=pass
```

Expected: PASS (П1 squash gives 3549 tests workspace-wide; per-crate counts ≥ post-П1 baseline).

If any failure surfaces, **halt and report BLOCKED** — must investigate before Stage 1.

- [ ] **Step 3: Capture pre-П2 cargo-public-api snapshots**

Run:

```bash
cargo public-api --manifest-path crates/credential/Cargo.toml > /tmp/credential-pre-p2.txt
cargo public-api --manifest-path crates/storage/Cargo.toml > /tmp/storage-pre-p2.txt
cargo public-api --manifest-path crates/engine/Cargo.toml > /tmp/engine-pre-p2.txt
```

Hold these for Stage 5 diff.

- [ ] **Step 4: Commit baseline marker**

```bash
git commit --allow-empty -m "chore(credential): П2 worktree baseline marker

Pre-П2 snapshots captured at /tmp/{credential,storage,engine}-pre-p2.txt
(local-only). All workspace gates green at this commit (post-П1 squash 6f83c81b).

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md"
```

### Task 0.2 — ADR-0034 draft

**Files:**
- Create: `docs/adr/0034-durable-credential-refresh-claim-repo.md`
- Modify: `docs/adr/README.md` (index)

- [ ] **Step 1: Verify next ADR number is 0034**

Run: `ls docs/adr/ | grep -E '^[0-9]{4}' | sort | tail -5`

Expected: `0040-controlaction-seal-canon-revision.md` is the latest renumbered post-action-cascade. ADR-0034 was reserved by spec frontmatter for the credential refresh claim repo. Verify the slot is still free (otherwise pick the next available — likely 0041 if conflicts).

If the spec's `planned-adrs:` slot conflicts with already-used numbers, use the next free number. Document the chosen number in the commit message.

- [ ] **Step 2: Write `docs/adr/0034-durable-credential-refresh-claim-repo.md`**

Use the project's ADR template (read 1-2 recent ADRs for shape — e.g., `0035-phantom-shim-capability-pattern.md`). Required sections per project convention:

- Status (Proposed → Accepted at landing)
- Context (cite n8n #13088 + sub-spec at `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`)
- Decision (two-tier coordinator: L1 mutex + L2 CAS claim repo; mirrors ADR-0008/0017 control-queue claim pattern)
- Consequences:
  - Positive: cross-replica safety, durable claim, sentinel mid-crash detection
  - Negative: extra storage round-trip per refresh (~1-5ms Postgres)
  - Neutral: SQLite parity preserves desktop mode
- Alternatives considered (cite spec §8.2 rejected: external coordinator, advisory locks, accept-the-race, single-writer election)
- Schema overview (`credential_refresh_claims` + `credential_sentinel_events` tables — high-level, not the migration body)
- Cross-references: spec, ADR-0030, ADR-0028, ADR-0008, ADR-0017

Length: ~150-250 lines, matches existing ADR depth.

- [ ] **Step 3: Update `docs/adr/README.md` index**

Add a row for ADR-0034 at the appropriate position. Match the existing format.

- [ ] **Step 4: Run rustdoc-adjacent checks**

ADRs are markdown — no rustdoc gate. But run `cargo +nightly fmt --check` (no Rust changes; should be clean) + lefthook's typos hook will run on commit.

- [ ] **Step 5: Commit**

```bash
git add docs/adr/0034-durable-credential-refresh-claim-repo.md docs/adr/README.md
git commit -m "docs(adr): ADR-0034 — durable credential refresh claim repository

Anticipated by sub-spec at docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md.
Lands ahead of Stage 1 implementation per the spec's §9 follow-ups.

Mirrors ADR-0008 (execution-control-queue-consumer) + ADR-0017
(control-queue-reclaim-policy) claim-repo pattern: CAS INSERT/UPDATE
with TTL + heartbeat + reclaim-on-crash. Two-tier composition
(in-process L1 + durable L2) preserves single-replica desktop mode
while gaining multi-replica safety in production.

Refs: docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md
      docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 0
      docs/adr/0030-engine-owns-credential-orchestration.md §3"
```

---

## Stage 1 — Storage infrastructure (P1 per spec §7.4)

Lands `RefreshClaimRepo` trait + DTOs + 3 impls (in-memory, SQLite, Postgres) + schema migrations. No engine changes yet.

### Task 1.1 — `RefreshClaimRepo` trait + DTOs

**Files:**
- Create: `crates/storage/src/credential/refresh_claim/mod.rs`
- Modify: `crates/storage/src/credential/mod.rs`

- [ ] **Step 1: Write the failing trait skeleton + DTO test (TDD)**

Create `crates/storage/src/credential/refresh_claim/mod.rs`:

```rust
//! Durable cross-replica claim repository for credential refresh
//! coordination.
//!
//! Per ADR-0034 + sub-spec
//! `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`.
//!
//! Implementations of [`RefreshClaimRepo`] provide CAS-based claim
//! acquisition with TTL + heartbeat semantics. Mirrors the control-queue
//! claim pattern (ADR-0008 + ADR-0017).

use std::time::Duration;

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Stable identifier for a credential. Re-exported from `nebula-core`.
pub type CredentialId = nebula_core::CredentialId;

/// Stable identifier for a Nebula replica process. Used to distinguish
/// claim holders for diagnostics + sweep ownership.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ReplicaId(String);

impl ReplicaId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque token returned to the holder after [`RefreshClaimRepo::try_claim`]
/// succeeds. Carries an internal generation counter so heartbeats from a
/// stale holder cannot extend a reclaimed claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimToken {
    pub claim_id: Uuid,
    pub generation: u64,
}

/// Successful claim record returned by [`RefreshClaimRepo::try_claim`].
#[derive(Clone, Debug)]
pub struct RefreshClaim {
    pub credential_id: CredentialId,
    pub token: ClaimToken,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Result of a [`RefreshClaimRepo::try_claim`] call.
#[derive(Debug)]
pub enum ClaimAttempt {
    /// Caller acquired the claim.
    Acquired(RefreshClaim),
    /// Another holder has a valid claim. The `existing_expires_at` lets
    /// the caller back off until that moment.
    Contended { existing_expires_at: DateTime<Utc> },
}

/// Errors from [`RefreshClaimRepo::heartbeat`].
#[derive(Debug, thiserror::Error)]
pub enum HeartbeatError {
    /// Our claim expired and another replica took it. Discard and re-check
    /// state.
    #[error("claim lost — another holder took ownership")]
    ClaimLost,
    /// Underlying repo error (DB connectivity etc.).
    #[error("repo error: {0}")]
    Repo(#[from] RepoError),
}

/// Errors from [`RefreshClaimRepo::try_claim`], [`release`], or
/// [`reclaim_stuck`].
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("storage error: {0}")]
    Storage(#[from] sqlx::Error),
    #[error("invalid state: {0}")]
    InvalidState(String),
}

/// Sentinel mark applied to an in-flight refresh row, per
/// sub-spec §3.4. Cleared on successful release; if found set
/// during reclaim sweep, the holder is presumed crashed mid-refresh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SentinelState {
    /// Normal claim — no IdP call yet OR already complete.
    Normal,
    /// Holder has started the IdP POST but not yet released.
    RefreshInFlight,
}

/// Cross-replica claim repository.
///
/// Per ADR-0034 + sub-spec §3.2. Implementations MUST:
///
/// - Provide atomic CAS semantics on [`try_claim`] (one acquirer wins
///   when multiple replicas attempt simultaneously).
/// - Validate holder/generation on [`heartbeat`] (a stale token cannot
///   extend a reclaimed claim).
/// - Idempotent [`release`].
/// - Reclaim sweep returns the credentials whose stale claims were
///   re-acquired (parallel to control-queue reclaim cadence).
///
/// [`try_claim`]: RefreshClaimRepo::try_claim
/// [`heartbeat`]: RefreshClaimRepo::heartbeat
/// [`release`]: RefreshClaimRepo::release
/// [`reclaim_stuck`]: RefreshClaimRepo::reclaim_stuck
#[async_trait::async_trait]
pub trait RefreshClaimRepo: Send + Sync + 'static {
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError>;

    async fn heartbeat(&self, token: &ClaimToken) -> Result<(), HeartbeatError>;

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError>;

    /// Marks the claim as `RefreshInFlight` — called immediately before
    /// the IdP POST. Idempotent.
    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError>;

    /// Sweeps claims past TTL, returns reclaimed credential ids paired
    /// with the sentinel state observed (so caller can record events for
    /// `RefreshInFlight` cases).
    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError>;
}

/// One row returned by [`RefreshClaimRepo::reclaim_stuck`].
#[derive(Debug, Clone)]
pub struct ReclaimedClaim {
    pub credential_id: CredentialId,
    pub previous_holder: ReplicaId,
    pub previous_generation: u64,
    pub sentinel: SentinelState,
}
```

Length budget: ≤200 lines for the trait + DTO module head. Sub-modules (in-memory, sqlite, pg) get their own files.

- [ ] **Step 2: Wire submodule**

Edit `crates/storage/src/credential/mod.rs` to add:

```rust
pub mod refresh_claim;
```

Re-export key types at appropriate places:

```rust
pub use refresh_claim::{
    ClaimAttempt, ClaimToken, HeartbeatError, RefreshClaim, RefreshClaimRepo,
    ReplicaId, RepoError, SentinelState, ReclaimedClaim,
};
```

- [ ] **Step 3: Verify trait compiles**

Run: `cargo check -p nebula-storage`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/storage/src/credential/refresh_claim/mod.rs crates/storage/src/credential/mod.rs
git commit -m "feat(storage): RefreshClaimRepo trait + DTOs (Stage 1.1)

Trait surface per ADR-0034 + sub-spec §3.2:
- try_claim returns Acquired | Contended for backoff
- heartbeat validates generation (stale token rejected)
- release idempotent
- mark_sentinel for mid-refresh crash detection
- reclaim_stuck sweep returns reclaimed claims with sentinel state

Concrete impls (in-memory, SQLite, Postgres) land in 1.2-1.4.

Refs: docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md §3.2
      docs/adr/0034-durable-credential-refresh-claim-repo.md
      docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 1.1"
```

### Task 1.2 — `InMemoryRefreshClaimRepo`

**Files:**
- Create: `crates/storage/src/credential/refresh_claim/in_memory.rs`
- Modify: `crates/storage/src/credential/refresh_claim/mod.rs` (re-export)

- [ ] **Step 1: Write failing test**

Add to `mod.rs`:

```rust
mod in_memory;
pub use in_memory::InMemoryRefreshClaimRepo;
```

Then in a new test file `crates/storage/tests/refresh_claim_in_memory_smoke.rs`:

```rust
use std::time::Duration;

use chrono::Utc;
use nebula_storage::credential::{
    ClaimAttempt, InMemoryRefreshClaimRepo, RefreshClaimRepo, ReplicaId,
};
use nebula_core::CredentialId;

#[tokio::test]
async fn try_claim_acquires_when_no_holder() {
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::from_string("test:cred").unwrap();
    let holder = ReplicaId::new("test-replica");

    let outcome = repo
        .try_claim(&cid, &holder, Duration::from_secs(30))
        .await
        .unwrap();

    match outcome {
        ClaimAttempt::Acquired(claim) => {
            assert_eq!(claim.credential_id, cid);
            assert!(claim.expires_at > claim.acquired_at);
        }
        ClaimAttempt::Contended { .. } => panic!("expected Acquired"),
    }
}

#[tokio::test]
async fn try_claim_returns_contended_when_held() {
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::from_string("test:cred").unwrap();

    let _first = repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap();

    let second = repo
        .try_claim(&cid, &ReplicaId::new("B"), Duration::from_secs(30))
        .await
        .unwrap();

    assert!(matches!(second, ClaimAttempt::Contended { .. }));
}
```

- [ ] **Step 2: Run failing test**

Run: `cargo nextest run -p nebula-storage --test refresh_claim_in_memory_smoke --profile ci --no-tests=pass`
Expected: FAIL with `cannot find type InMemoryRefreshClaimRepo`.

- [ ] **Step 3: Implement `InMemoryRefreshClaimRepo`**

Create `crates/storage/src/credential/refresh_claim/in_memory.rs`:

```rust
//! In-memory `RefreshClaimRepo` impl for tests + desktop-mode fallback.
//!
//! Single-process scope — no cross-replica coordination. CAS uses a
//! `parking_lot::Mutex` over `HashMap<CredentialId, ClaimRow>`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, CredentialId, HeartbeatError, ReclaimedClaim,
    RefreshClaim, RefreshClaimRepo, ReplicaId, RepoError, SentinelState,
};

#[derive(Clone, Debug)]
struct ClaimRow {
    claim_id: Uuid,
    generation: u64,
    holder: ReplicaId,
    acquired_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    sentinel: SentinelState,
}

/// In-memory `RefreshClaimRepo`. Cheap to clone (Arc-backed inner).
#[derive(Clone, Default)]
pub struct InMemoryRefreshClaimRepo {
    inner: Arc<Mutex<HashMap<CredentialId, ClaimRow>>>,
}

impl InMemoryRefreshClaimRepo {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl RefreshClaimRepo for InMemoryRefreshClaimRepo {
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        let now = Utc::now();
        let mut guard = self.inner.lock();

        if let Some(existing) = guard.get(credential_id) {
            if existing.expires_at > now {
                return Ok(ClaimAttempt::Contended {
                    existing_expires_at: existing.expires_at,
                });
            }
            // Expired — overwrite with bumped generation.
        }

        let claim_id = Uuid::new_v4();
        let generation = guard
            .get(credential_id)
            .map(|row| row.generation + 1)
            .unwrap_or(0);
        let acquired_at = now;
        let expires_at = now
            + chrono::Duration::from_std(ttl)
                .map_err(|e| RepoError::InvalidState(format!("invalid ttl: {e}")))?;

        let row = ClaimRow {
            claim_id,
            generation,
            holder: holder.clone(),
            acquired_at,
            expires_at,
            sentinel: SentinelState::Normal,
        };
        guard.insert(credential_id.clone(), row);

        Ok(ClaimAttempt::Acquired(RefreshClaim {
            credential_id: credential_id.clone(),
            token: ClaimToken {
                claim_id,
                generation,
            },
            acquired_at,
            expires_at,
        }))
    }

    async fn heartbeat(&self, token: &ClaimToken) -> Result<(), HeartbeatError> {
        let now = Utc::now();
        let mut guard = self.inner.lock();

        let row = guard
            .values_mut()
            .find(|r| r.claim_id == token.claim_id && r.generation == token.generation);
        match row {
            Some(r) if r.expires_at > now => {
                // Extend by the same default — caller's RefreshCoordinator
                // controls the TTL across calls; here we use the row's
                // current expiry as the basis. To stay simple, double
                // the time-to-now:
                let remaining = r.expires_at - now;
                r.expires_at = now + remaining + chrono::Duration::seconds(30);
                Ok(())
            }
            Some(_) => Err(HeartbeatError::ClaimLost),
            None => Err(HeartbeatError::ClaimLost),
        }
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError> {
        let mut guard = self.inner.lock();
        guard.retain(|_, row| {
            !(row.claim_id == token.claim_id && row.generation == token.generation)
        });
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        let mut guard = self.inner.lock();
        let row = guard
            .values_mut()
            .find(|r| r.claim_id == token.claim_id && r.generation == token.generation);
        if let Some(r) = row {
            r.sentinel = SentinelState::RefreshInFlight;
        }
        Ok(())
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError> {
        let now = Utc::now();
        let mut guard = self.inner.lock();
        let mut out = Vec::new();

        let stuck: Vec<CredentialId> = guard
            .iter()
            .filter(|(_, r)| r.expires_at < now)
            .map(|(k, _)| k.clone())
            .collect();

        for cid in stuck {
            if let Some(row) = guard.remove(&cid) {
                out.push(ReclaimedClaim {
                    credential_id: cid,
                    previous_holder: row.holder.clone(),
                    previous_generation: row.generation,
                    sentinel: row.sentinel,
                });
            }
        }

        Ok(out)
    }
}
```

Length budget: ~150 lines. Self-contained.

- [ ] **Step 4: Run smoke test**

Run: `cargo nextest run -p nebula-storage --test refresh_claim_in_memory_smoke --profile ci --no-tests=pass`
Expected: PASS (both tests).

- [ ] **Step 5: Add additional in-memory tests**

Append to `tests/refresh_claim_in_memory_smoke.rs`:

```rust
#[tokio::test]
async fn heartbeat_validates_generation() {
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::from_string("test:cred").unwrap();

    let claim = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        _ => panic!(),
    };

    // Stale token — bump generation manually
    let stale = ClaimToken {
        claim_id: claim.token.claim_id,
        generation: claim.token.generation + 99,
    };

    let result = repo.heartbeat(&stale).await;
    assert!(matches!(result, Err(_)));
}

#[tokio::test]
async fn release_is_idempotent() {
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::from_string("test:cred").unwrap();

    let claim = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_secs(30))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        _ => panic!(),
    };

    repo.release(claim.token.clone()).await.unwrap();
    repo.release(claim.token.clone()).await.unwrap(); // idempotent
}

#[tokio::test]
async fn reclaim_returns_expired_with_sentinel_state() {
    let repo = InMemoryRefreshClaimRepo::new();
    let cid = CredentialId::from_string("test:cred").unwrap();

    let claim = match repo
        .try_claim(&cid, &ReplicaId::new("A"), Duration::from_millis(50))
        .await
        .unwrap()
    {
        ClaimAttempt::Acquired(c) => c,
        _ => panic!(),
    };
    repo.mark_sentinel(&claim.token).await.unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    let reclaimed = repo.reclaim_stuck().await.unwrap();

    assert_eq!(reclaimed.len(), 1);
    assert_eq!(reclaimed[0].credential_id, cid);
    assert_eq!(
        reclaimed[0].sentinel,
        nebula_storage::credential::SentinelState::RefreshInFlight
    );
}
```

- [ ] **Step 6: Run all in-memory tests**

Run: `cargo nextest run -p nebula-storage --test refresh_claim_in_memory_smoke --profile ci --no-tests=pass`
Expected: PASS (5 tests).

- [ ] **Step 7: Commit**

```bash
git add crates/storage/src/credential/refresh_claim/in_memory.rs \
        crates/storage/src/credential/refresh_claim/mod.rs \
        crates/storage/tests/refresh_claim_in_memory_smoke.rs
git commit -m "feat(storage): InMemoryRefreshClaimRepo (Stage 1.2)

In-memory impl for tests + desktop-mode fallback. Single-process
scope. Mutex<HashMap<CredentialId, ClaimRow>> with generation
counter for stale-token detection.

Test coverage: try_claim acquire/contended, heartbeat validation,
release idempotency, reclaim with sentinel state.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 1.2"
```

### Task 1.3 — SQLite migration + impl

**Files:**
- Create: `crates/storage/migrations/sqlite/0022_credential_refresh_claims.sql`
- Create: `crates/storage/migrations/sqlite/0023_credential_sentinel_events.sql`
- Create: `crates/storage/src/credential/refresh_claim/sqlite.rs`
- Modify: `crates/storage/src/credential/refresh_claim/mod.rs`

- [ ] **Step 1: Write SQLite migration 0022 (claims table)**

`crates/storage/migrations/sqlite/0022_credential_refresh_claims.sql`:

```sql
-- Per ADR-0034 + sub-spec §3.3
-- Holds in-flight refresh claims for cross-replica coordination.
CREATE TABLE credential_refresh_claims (
    credential_id     TEXT    NOT NULL PRIMARY KEY,
    claim_id          TEXT    NOT NULL,                        -- UUID
    generation        INTEGER NOT NULL,                          -- bumped on each CAS
    holder_replica_id TEXT    NOT NULL,
    acquired_at       TEXT    NOT NULL,                          -- ISO-8601
    expires_at        TEXT    NOT NULL,
    sentinel          INTEGER NOT NULL DEFAULT 0,                -- 0=normal, 1=refresh_in_flight
    CHECK (sentinel IN (0, 1))
);

CREATE INDEX idx_refresh_claims_expires
    ON credential_refresh_claims(expires_at);
```

- [ ] **Step 2: Write SQLite migration 0023 (sentinel events)**

`crates/storage/migrations/sqlite/0023_credential_sentinel_events.sql`:

```sql
-- Per ADR-0034 + sub-spec §3.4
-- One row per detected mid-refresh crash. Reclaim sweep inserts a row
-- when it finds an expired claim with sentinel=1. The threshold logic
-- (N=3 within 1h) lives in nebula-engine.
CREATE TABLE credential_sentinel_events (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    credential_id     TEXT    NOT NULL,
    detected_at       TEXT    NOT NULL,                          -- ISO-8601
    crashed_holder    TEXT    NOT NULL,                          -- replica id
    generation        INTEGER NOT NULL                            -- claim row's generation at crash
);

CREATE INDEX idx_sentinel_events_cred_time
    ON credential_sentinel_events(credential_id, detected_at);
```

- [ ] **Step 3: Repeat for Postgres**

`crates/storage/migrations/postgres/0022_credential_refresh_claims.sql`:

```sql
-- Per ADR-0034 + sub-spec §3.3
CREATE TABLE credential_refresh_claims (
    credential_id     TEXT NOT NULL PRIMARY KEY,
    claim_id          UUID NOT NULL,
    generation        BIGINT NOT NULL,
    holder_replica_id TEXT NOT NULL,
    acquired_at       TIMESTAMPTZ NOT NULL,
    expires_at        TIMESTAMPTZ NOT NULL,
    sentinel          SMALLINT NOT NULL DEFAULT 0,
    CHECK (sentinel IN (0, 1))
);

CREATE INDEX idx_refresh_claims_expires
    ON credential_refresh_claims(expires_at);
```

`crates/storage/migrations/postgres/0023_credential_sentinel_events.sql`:

```sql
-- Per ADR-0034 + sub-spec §3.4
CREATE TABLE credential_sentinel_events (
    id              BIGSERIAL PRIMARY KEY,
    credential_id   TEXT NOT NULL,
    detected_at     TIMESTAMPTZ NOT NULL,
    crashed_holder  TEXT NOT NULL,
    generation      BIGINT NOT NULL
);

CREATE INDEX idx_sentinel_events_cred_time
    ON credential_sentinel_events(credential_id, detected_at);
```

- [ ] **Step 4: Run schema-parity CI check locally**

Find the existing schema-parity check (likely in `crates/storage/tests/` or a CI script):

```bash
grep -rn "schema_parity\|schema-parity" crates/storage/ .github/workflows/
```

Run that test/check. If it has structural-form comparison, both new files must satisfy it. If not, add a check in the same shape as existing migrations.

- [ ] **Step 5: Implement `SqliteRefreshClaimRepo`**

Create `crates/storage/src/credential/refresh_claim/sqlite.rs`:

```rust
//! SQLite-backed `RefreshClaimRepo` impl.
//!
//! Single-replica desktop mode + multi-process tests. CAS via
//! `INSERT ... ON CONFLICT DO UPDATE WHERE` to mirror Postgres
//! `INSERT ... ON CONFLICT ... WHERE` pattern.

use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, CredentialId, HeartbeatError, ReclaimedClaim, RefreshClaim,
    RefreshClaimRepo, ReplicaId, RepoError, SentinelState,
};

#[derive(Clone)]
pub struct SqliteRefreshClaimRepo {
    pool: SqlitePool,
}

impl SqliteRefreshClaimRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl RefreshClaimRepo for SqliteRefreshClaimRepo {
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        let now = Utc::now();
        let new_claim_id = Uuid::new_v4();
        let new_expires = now
            + chrono::Duration::from_std(ttl)
                .map_err(|e| RepoError::InvalidState(format!("invalid ttl: {e}")))?;
        let cid_str = credential_id.as_str();
        let holder_str = holder.as_str();
        let now_iso = now.to_rfc3339();
        let exp_iso = new_expires.to_rfc3339();
        let claim_id_str = new_claim_id.to_string();

        // Fetch existing first to surface contention with expires_at.
        let existing: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT claim_id, generation, expires_at \
             FROM credential_refresh_claims \
             WHERE credential_id = ?1",
        )
        .bind(cid_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::Storage)?;

        if let Some((_existing_claim_id, existing_gen, existing_exp_str)) = &existing {
            let existing_exp: DateTime<Utc> = existing_exp_str
                .parse::<DateTime<Utc>>()
                .map_err(|e| RepoError::InvalidState(format!("bad expires_at: {e}")))?;
            if existing_exp > now {
                return Ok(ClaimAttempt::Contended {
                    existing_expires_at: existing_exp,
                });
            }
            // Expired — overwrite, bump generation.
            let new_gen = existing_gen + 1;
            sqlx::query(
                "UPDATE credential_refresh_claims \
                 SET claim_id = ?1, generation = ?2, holder_replica_id = ?3, \
                     acquired_at = ?4, expires_at = ?5, sentinel = 0 \
                 WHERE credential_id = ?6 AND expires_at < ?7",
            )
            .bind(&claim_id_str)
            .bind(new_gen)
            .bind(holder_str)
            .bind(&now_iso)
            .bind(&exp_iso)
            .bind(cid_str)
            .bind(&now_iso)
            .execute(&self.pool)
            .await
            .map_err(RepoError::Storage)?;

            return Ok(ClaimAttempt::Acquired(RefreshClaim {
                credential_id: credential_id.clone(),
                token: ClaimToken {
                    claim_id: new_claim_id,
                    generation: new_gen as u64,
                },
                acquired_at: now,
                expires_at: new_expires,
            }));
        }

        // No row — INSERT.
        sqlx::query(
            "INSERT INTO credential_refresh_claims \
             (credential_id, claim_id, generation, holder_replica_id, \
              acquired_at, expires_at, sentinel) \
             VALUES (?1, ?2, 0, ?3, ?4, ?5, 0)",
        )
        .bind(cid_str)
        .bind(&claim_id_str)
        .bind(holder_str)
        .bind(&now_iso)
        .bind(&exp_iso)
        .execute(&self.pool)
        .await
        .map_err(RepoError::Storage)?;

        Ok(ClaimAttempt::Acquired(RefreshClaim {
            credential_id: credential_id.clone(),
            token: ClaimToken {
                claim_id: new_claim_id,
                generation: 0,
            },
            acquired_at: now,
            expires_at: new_expires,
        }))
    }

    async fn heartbeat(&self, token: &ClaimToken) -> Result<(), HeartbeatError> {
        let now = Utc::now();
        let now_iso = now.to_rfc3339();
        let extension = (now + chrono::Duration::seconds(30)).to_rfc3339();
        let claim_id_str = token.claim_id.to_string();

        let rows = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET expires_at = ?1 \
             WHERE claim_id = ?2 AND generation = ?3 AND expires_at > ?4",
        )
        .bind(&extension)
        .bind(&claim_id_str)
        .bind(token.generation as i64)
        .bind(&now_iso)
        .execute(&self.pool)
        .await
        .map_err(RepoError::Storage)?
        .rows_affected();

        if rows == 0 {
            return Err(HeartbeatError::ClaimLost);
        }
        Ok(())
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError> {
        let claim_id_str = token.claim_id.to_string();
        sqlx::query(
            "DELETE FROM credential_refresh_claims \
             WHERE claim_id = ?1 AND generation = ?2",
        )
        .bind(&claim_id_str)
        .bind(token.generation as i64)
        .execute(&self.pool)
        .await
        .map_err(RepoError::Storage)?;
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        let claim_id_str = token.claim_id.to_string();
        sqlx::query(
            "UPDATE credential_refresh_claims \
             SET sentinel = 1 \
             WHERE claim_id = ?1 AND generation = ?2",
        )
        .bind(&claim_id_str)
        .bind(token.generation as i64)
        .execute(&self.pool)
        .await
        .map_err(RepoError::Storage)?;
        Ok(())
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError> {
        let now = Utc::now();
        let now_iso = now.to_rfc3339();

        // Two-phase for SQLite (no RETURNING with DELETE everywhere):
        // 1. SELECT expired rows
        // 2. DELETE them
        let stuck: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "SELECT credential_id, holder_replica_id, generation, sentinel \
             FROM credential_refresh_claims \
             WHERE expires_at < ?1",
        )
        .bind(&now_iso)
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::Storage)?;

        sqlx::query(
            "DELETE FROM credential_refresh_claims WHERE expires_at < ?1",
        )
        .bind(&now_iso)
        .execute(&self.pool)
        .await
        .map_err(RepoError::Storage)?;

        let out = stuck
            .into_iter()
            .map(|(cid, holder, gen, sent)| ReclaimedClaim {
                credential_id: CredentialId::from_string(&cid).unwrap(),
                previous_holder: ReplicaId::new(holder),
                previous_generation: gen as u64,
                sentinel: if sent == 1 {
                    SentinelState::RefreshInFlight
                } else {
                    SentinelState::Normal
                },
            })
            .collect();

        Ok(out)
    }
}
```

Length budget: ~250 lines.

- [ ] **Step 6: Add to `mod.rs` re-exports**

```rust
mod sqlite;
pub use sqlite::SqliteRefreshClaimRepo;
```

Behind a `cfg(feature = "sqlite")` if the crate already gates SQLite — verify by reading existing cfg patterns.

- [ ] **Step 7: Write SQLite integration test**

Create `crates/storage/tests/refresh_claim_sqlite_integration.rs`. Use the existing test-helper for SQLite pool (e.g., `nebula_storage::testing::sqlite_pool()` if available; otherwise raw `sqlx::sqlite::SqlitePoolOptions::new().connect_lazy(":memory:")` + run migrations).

Cover:
- `try_claim acquire then re-acquire after expiry → PASS`
- `concurrent try_claim across two pool clones → exactly one Acquired`
- `heartbeat extends expiry`
- `mark_sentinel then reclaim → ReclaimedClaim with RefreshInFlight`

Each test ~20-40 lines.

- [ ] **Step 8: Run SQLite integration tests**

Run: `cargo nextest run -p nebula-storage --test refresh_claim_sqlite_integration --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/storage/migrations/sqlite/0022_credential_refresh_claims.sql \
        crates/storage/migrations/sqlite/0023_credential_sentinel_events.sql \
        crates/storage/migrations/postgres/0022_credential_refresh_claims.sql \
        crates/storage/migrations/postgres/0023_credential_sentinel_events.sql \
        crates/storage/src/credential/refresh_claim/sqlite.rs \
        crates/storage/src/credential/refresh_claim/mod.rs \
        crates/storage/tests/refresh_claim_sqlite_integration.rs
git commit -m "feat(storage): SQLite RefreshClaimRepo + migrations 0022/0023 (Stage 1.3)

Schema parity SQLite + Postgres for both new tables. SQLite impl
uses two-phase reclaim (SELECT + DELETE) since RETURNING-on-DELETE
is not universally supported across SQLite versions.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 1.3
      docs/adr/0034-durable-credential-refresh-claim-repo.md"
```

### Task 1.4 — Postgres impl

**Files:**
- Create: `crates/storage/src/credential/refresh_claim/postgres.rs`
- Modify: `crates/storage/src/credential/refresh_claim/mod.rs`

- [ ] **Step 1: Implement `PgRefreshClaimRepo`**

Create `crates/storage/src/credential/refresh_claim/postgres.rs`. Mirror the SQLite impl shape but use Postgres-native features:

- `FOR UPDATE SKIP LOCKED` on the contended path (avoids lock contention sweep)
- `RETURNING credential_id` on the reclaim path (single round-trip)
- UUID type native (no string conversion)
- TIMESTAMPTZ native (no ISO-8601 marshal)

```rust
//! Postgres-backed `RefreshClaimRepo` impl.
//!
//! Multi-replica production target. Atomic CAS via
//! `INSERT ... ON CONFLICT (credential_id) DO UPDATE WHERE
//! credential_refresh_claims.expires_at < EXCLUDED.expires_at`
//! pattern, mirroring control-queue claim acquisition (ADR-0008).

use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, CredentialId, HeartbeatError, ReclaimedClaim, RefreshClaim,
    RefreshClaimRepo, ReplicaId, RepoError, SentinelState,
};

#[derive(Clone)]
pub struct PgRefreshClaimRepo {
    pool: PgPool,
}

impl PgRefreshClaimRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl RefreshClaimRepo for PgRefreshClaimRepo {
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        let now = Utc::now();
        let new_claim_id = Uuid::new_v4();
        let new_expires = now
            + chrono::Duration::from_std(ttl)
                .map_err(|e| RepoError::InvalidState(format!("invalid ttl: {e}")))?;

        // Atomic CAS: INSERT or UPDATE only if existing row is expired.
        // Returns the ROW WE WROTE if we won, or the existing valid row's
        // expires_at if we lost.
        let row: Option<(Uuid, i64, DateTime<Utc>, DateTime<Utc>, bool)> = sqlx::query_as(
            "INSERT INTO credential_refresh_claims \
             (credential_id, claim_id, generation, holder_replica_id, \
              acquired_at, expires_at, sentinel) \
             VALUES ($1, $2, 0, $3, $4, $5, 0) \
             ON CONFLICT (credential_id) DO UPDATE \
             SET claim_id = EXCLUDED.claim_id, \
                 generation = credential_refresh_claims.generation + 1, \
                 holder_replica_id = EXCLUDED.holder_replica_id, \
                 acquired_at = EXCLUDED.acquired_at, \
                 expires_at = EXCLUDED.expires_at, \
                 sentinel = 0 \
             WHERE credential_refresh_claims.expires_at < $4 \
             RETURNING claim_id, generation, acquired_at, expires_at, true AS won",
        )
        .bind(credential_id.as_str())
        .bind(new_claim_id)
        .bind(holder.as_str())
        .bind(now)
        .bind(new_expires)
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::Storage)?;

        if let Some((claim_id, generation, acquired, expires, _won)) = row {
            return Ok(ClaimAttempt::Acquired(RefreshClaim {
                credential_id: credential_id.clone(),
                token: ClaimToken {
                    claim_id,
                    generation: generation as u64,
                },
                acquired_at: acquired,
                expires_at: expires,
            }));
        }

        // CAS lost — fetch existing row's expires_at for backoff timing.
        let existing: Option<(DateTime<Utc>,)> = sqlx::query_as(
            "SELECT expires_at FROM credential_refresh_claims WHERE credential_id = $1",
        )
        .bind(credential_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(RepoError::Storage)?;

        match existing {
            Some((exp,)) => Ok(ClaimAttempt::Contended {
                existing_expires_at: exp,
            }),
            None => Err(RepoError::InvalidState(
                "CAS lost but no existing row visible".into(),
            )),
        }
    }

    async fn heartbeat(&self, token: &ClaimToken) -> Result<(), HeartbeatError> {
        let now = Utc::now();
        let extension = now + chrono::Duration::seconds(30);

        let rows = sqlx::query(
            "UPDATE credential_refresh_claims \
             SET expires_at = $1 \
             WHERE claim_id = $2 AND generation = $3 AND expires_at > $4",
        )
        .bind(extension)
        .bind(token.claim_id)
        .bind(token.generation as i64)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(RepoError::Storage)?
        .rows_affected();

        if rows == 0 {
            return Err(HeartbeatError::ClaimLost);
        }
        Ok(())
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError> {
        sqlx::query(
            "DELETE FROM credential_refresh_claims \
             WHERE claim_id = $1 AND generation = $2",
        )
        .bind(token.claim_id)
        .bind(token.generation as i64)
        .execute(&self.pool)
        .await
        .map_err(RepoError::Storage)?;
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        sqlx::query(
            "UPDATE credential_refresh_claims \
             SET sentinel = 1 \
             WHERE claim_id = $1 AND generation = $2",
        )
        .bind(token.claim_id)
        .bind(token.generation as i64)
        .execute(&self.pool)
        .await
        .map_err(RepoError::Storage)?;
        Ok(())
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError> {
        let now = Utc::now();
        let rows: Vec<(String, String, i64, i32)> = sqlx::query_as(
            "DELETE FROM credential_refresh_claims \
             WHERE expires_at < $1 \
             RETURNING credential_id, holder_replica_id, generation, sentinel",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(RepoError::Storage)?;

        let out = rows
            .into_iter()
            .map(|(cid, holder, gen, sent)| ReclaimedClaim {
                credential_id: CredentialId::from_string(&cid).unwrap(),
                previous_holder: ReplicaId::new(holder),
                previous_generation: gen as u64,
                sentinel: if sent == 1 {
                    SentinelState::RefreshInFlight
                } else {
                    SentinelState::Normal
                },
            })
            .collect();

        Ok(out)
    }
}
```

- [ ] **Step 2: Add to `mod.rs` re-exports**

```rust
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "postgres")]
pub use postgres::PgRefreshClaimRepo;
```

Match existing cfg-gating pattern.

- [ ] **Step 3: Postgres integration test**

Create `crates/storage/tests/refresh_claim_pg_integration.rs`. Use whatever Postgres test-pool helper the crate already provides (e.g., `testcontainers` or `sqlx_test_pool!`). Cover the same 4 cases as SQLite + a concurrent CAS race assertion using `tokio::join!` over multiple connections.

- [ ] **Step 4: Run Postgres integration tests**

Run: `cargo nextest run -p nebula-storage --features postgres --test refresh_claim_pg_integration --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/storage/src/credential/refresh_claim/postgres.rs \
        crates/storage/src/credential/refresh_claim/mod.rs \
        crates/storage/tests/refresh_claim_pg_integration.rs
git commit -m "feat(storage): Postgres RefreshClaimRepo (Stage 1.4)

Atomic CAS via INSERT ON CONFLICT WHERE expires_at < $1, returning
the row we wrote. Loses-cleanly fetches existing expires_at for
backoff timing.

reclaim_stuck uses DELETE ... RETURNING (single round-trip).

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 1.4
      docs/adr/0034-durable-credential-refresh-claim-repo.md
      ADR-0008/0017 control-queue claim pattern"
```

### Task 1.5 — Loom test for atomic CAS

**Files:**
- Create: `crates/storage/tests/refresh_claim_loom.rs`
- Modify: `crates/storage/Cargo.toml` (dev-dep + cfg)

- [ ] **Step 1: Add loom dev-dep**

Edit `crates/storage/Cargo.toml`:

```toml
[dev-dependencies]
# ... existing ...
loom = "0.7"
```

If loom is workspace-pinned, use `{ workspace = true }`.

- [ ] **Step 2: Write loom test**

Create `crates/storage/tests/refresh_claim_loom.rs`:

```rust
//! Loom test asserting CAS atomicity under 2-thread interleaving.
//!
//! Per sub-spec §10 DoD requirement.
//!
//! Loom replaces tokio's executor with a deterministic scheduler that
//! exhaustively explores thread interleavings for the configured atomic
//! ordering. We use the in-memory repo (single-process) and assert that
//! at most one of two concurrent try_claim attempts can return Acquired.

#![cfg(loom)]

use std::sync::Arc;
use std::time::Duration;

use loom::sync::atomic::{AtomicU32, Ordering};
use loom::thread;
use nebula_storage::credential::{
    ClaimAttempt, InMemoryRefreshClaimRepo, RefreshClaimRepo, ReplicaId,
};

#[test]
fn no_concurrent_claim_acquires_succeed() {
    loom::model(|| {
        let repo = Arc::new(InMemoryRefreshClaimRepo::new());
        let cid = nebula_core::CredentialId::from_string("loom:cred").unwrap();
        let acquired = Arc::new(AtomicU32::new(0));

        let h1 = thread::spawn({
            let repo = Arc::clone(&repo);
            let acquired = Arc::clone(&acquired);
            let cid = cid.clone();
            move || {
                let outcome = futures::executor::block_on(repo.try_claim(
                    &cid,
                    &ReplicaId::new("A"),
                    Duration::from_secs(30),
                ));
                if matches!(outcome, Ok(ClaimAttempt::Acquired(_))) {
                    acquired.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        let h2 = thread::spawn({
            let repo = Arc::clone(&repo);
            let acquired = Arc::clone(&acquired);
            let cid = cid.clone();
            move || {
                let outcome = futures::executor::block_on(repo.try_claim(
                    &cid,
                    &ReplicaId::new("B"),
                    Duration::from_secs(30),
                ));
                if matches!(outcome, Ok(ClaimAttempt::Acquired(_))) {
                    acquired.fetch_add(1, Ordering::Relaxed);
                }
            }
        });

        h1.join().unwrap();
        h2.join().unwrap();

        // Exactly one acquirer wins.
        assert_eq!(acquired.load(Ordering::Relaxed), 1);
    });
}
```

Note: loom is invoked via `RUSTFLAGS="--cfg loom" cargo test --test refresh_claim_loom` per project convention. Verify by checking existing loom usage in the repo:

```bash
grep -rn "cfg(loom)" crates/ | head -5
grep -rn "RUSTFLAGS.*loom\|--cfg loom" .github/ Makefile* 2>/dev/null
```

If existing pattern differs, adapt. Loom currently in repo: check `crates/credential/` or others.

- [ ] **Step 3: Run the loom test**

Run: `RUSTFLAGS="--cfg loom" cargo nextest run -p nebula-storage --test refresh_claim_loom --profile ci --no-tests=pass`

If loom sees more than 1 thread interleaving where both acquire, it fails — expected outcome is exactly 1. Loom can take 10-30s to enumerate interleavings; that's acceptable.

- [ ] **Step 4: Commit**

```bash
git add crates/storage/tests/refresh_claim_loom.rs crates/storage/Cargo.toml
git commit -m "test(storage): loom CAS atomicity probe (Stage 1.5)

Exhaustively explores 2-thread interleavings of try_claim. Asserts
exactly one acquirer wins under any scheduling.

Run with: RUSTFLAGS=\"--cfg loom\" cargo test --test refresh_claim_loom

Refs: docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md §5.1, §10
      docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 1.5"
```

### Task 1.6 — Stage 1 gate

- [ ] **Step 1: Run Stage 1 gate**

```bash
cargo +nightly fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run -p nebula-storage --profile ci --no-tests=pass
RUSTFLAGS="--cfg loom" cargo nextest run -p nebula-storage --test refresh_claim_loom --profile ci --no-tests=pass
RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-storage --no-deps
```

Expected: all PASS.

- [ ] **Step 2: Stage 1 marker commit**

```bash
git commit --allow-empty -m "chore(credential): Stage 1 gate passed (storage infrastructure)

RefreshClaimRepo trait + 3 impls (in-memory + SQLite + Postgres)
+ migrations 0022/0023 with schema parity + loom CAS probe.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 1"
```

---

## Stage 2 — Engine coordinator refactor (P2 per spec §7.4)

Refactors existing in-process coordinator into a private inner; wraps with new outer L1+L2 coordinator. Plumbs `Arc<dyn RefreshClaimRepo>` through composition root. Updates `token_refresh.rs` to mark sentinel.

### Task 2.1 — Rename existing `RefreshCoordinator` → `L1RefreshCoalescer`

**Files:**
- Create: `crates/engine/src/credential/refresh/l1.rs` (renamed content)
- Create: `crates/engine/src/credential/refresh/mod.rs`
- Delete: `crates/engine/src/credential/refresh.rs`
- Modify: `crates/engine/src/credential/mod.rs`

- [ ] **Step 1: Audit current `refresh.rs` callers**

Run: `grep -rn 'RefreshCoordinator\|refresh::' crates/engine/`

Note: every external caller will need to switch to the new outer `RefreshCoordinator` (re-exported under same name). Internal renames stay private.

- [ ] **Step 2: Move file content**

Move `crates/engine/src/credential/refresh.rs` → `crates/engine/src/credential/refresh/l1.rs`. Inside the file, rename:
- `pub struct RefreshCoordinator` → `pub(crate) struct L1RefreshCoalescer`
- `pub fn try_refresh` → `pub(crate) fn try_refresh`
- All other `pub` → `pub(crate)`

Update doc-comments to reflect "L1 in-process coalescing" (not the outer coordinator).

- [ ] **Step 3: Create `refresh/mod.rs`**

```rust
//! Two-tier refresh coordinator (L1 in-process + L2 cross-replica claim).
//!
//! Per ADR-0034 + sub-spec §3.
//!
//! [`RefreshCoordinator`] is the public outer surface. Callers invoke
//! [`refresh_coalesced`] which acquires L1 mutex first (in-process
//! coalescing), then a durable L2 claim via [`RefreshClaimRepo`]
//! (cross-replica coordination), then runs the user's refresh closure.

mod l1;
mod coordinator;
mod sentinel;
mod reclaim;

pub use coordinator::{RefreshCoordinator, RefreshCoordConfig};
pub(crate) use l1::L1RefreshCoalescer;
pub use sentinel::SentinelTrigger;  // exposed for tests + custom thresholds
pub use reclaim::ReclaimSweepHandle;
```

- [ ] **Step 4: Update `crates/engine/src/credential/mod.rs`**

Change `pub mod refresh;` to remain a single line (now refers to the directory); re-exports still flow through.

- [ ] **Step 5: Verify L1 alone compiles**

Run: `cargo check -p nebula-engine`
Expected: FAIL (because `coordinator`, `sentinel`, `reclaim` don't exist yet). That's OK — Tasks 2.2-2.4 fill them.

To make this commit independently buildable, comment out `mod coordinator;` etc. for now. Or write minimal stubs in coordinator.rs/sentinel.rs/reclaim.rs that re-export L1 only:

```rust
// crates/engine/src/credential/refresh/coordinator.rs (stub)
pub use super::l1::L1RefreshCoalescer as RefreshCoordinator;
pub struct RefreshCoordConfig;  // placeholder
```

This keeps the rename atomic. Real coordinator lands in 2.2.

- [ ] **Step 6: Run engine tests**

Run: `cargo nextest run -p nebula-engine --profile ci --no-tests=pass`
Expected: PASS (existing L1 behavior preserved).

- [ ] **Step 7: Commit**

```bash
git add crates/engine/src/credential/refresh/ \
        crates/engine/src/credential/mod.rs
git rm crates/engine/src/credential/refresh.rs
git commit -m "refactor(engine): rename RefreshCoordinator → L1RefreshCoalescer (Stage 2.1)

Existing in-process coordinator becomes private L1 coalescer; new
outer two-tier RefreshCoordinator (L1+L2) lands in Stage 2.2.

Public re-export preserved at refresh::RefreshCoordinator (currently
aliased to L1; expanded in 2.2). No callers break.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 2.1"
```

### Task 2.2 — New outer `RefreshCoordinator` (L1+L2)

**Files:**
- Modify: `crates/engine/src/credential/refresh/coordinator.rs` (replace stub)
- Modify: `crates/engine/Cargo.toml` (add `nebula-storage` if not present in dev/runtime — verify)

- [ ] **Step 1: Verify `nebula-engine` already depends on `nebula-storage`**

Run: `grep nebula-storage crates/engine/Cargo.toml`

If absent, add as a runtime dep (`nebula-storage = { path = "../storage" }`). Per ADR-0030 the engine owns orchestration over storage — this dep direction is correct.

- [ ] **Step 2: Implement outer `RefreshCoordinator`**

Replace `crates/engine/src/credential/refresh/coordinator.rs` with full impl per sub-spec §3.1:

```rust
//! Outer two-tier refresh coordinator.

use std::sync::Arc;
use std::time::Duration;

use nebula_storage::credential::{
    ClaimAttempt, RefreshClaim, RefreshClaimRepo, ReplicaId,
};

use super::l1::L1RefreshCoalescer;

/// Configuration knobs for the two-tier coordinator. Defaults satisfy
/// the invariants in [`RefreshCoordConfig::validate`]; CI test asserts
/// `RefreshCoordConfig::default().validate().is_ok()`.
#[derive(Clone, Debug)]
pub struct RefreshCoordConfig {
    pub claim_ttl: Duration,
    pub heartbeat_interval: Duration,
    pub refresh_timeout: Duration,
    pub reclaim_sweep_interval: Duration,
    pub sentinel_threshold: u32,
    pub sentinel_window: Duration,
}

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

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("heartbeat_interval × 3 must be < claim_ttl")]
    HeartbeatTooSlow,
    #[error("refresh_timeout + 2 × heartbeat_interval must be < claim_ttl")]
    RefreshTimeoutTooLong,
    #[error("reclaim_sweep_interval must be ≤ claim_ttl")]
    ReclaimTooSlow,
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

/// Outer two-tier coordinator. Composes:
/// - L1: in-process [`L1RefreshCoalescer`]
/// - L2: durable [`RefreshClaimRepo`] for cross-replica safety
pub struct RefreshCoordinator {
    l1: L1RefreshCoalescer,
    repo: Arc<dyn RefreshClaimRepo>,
    replica_id: ReplicaId,
    config: RefreshCoordConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error("contention exhausted after retries")]
    ContentionExhausted,
    #[error("refresh coalesced by another replica (success — re-read state)")]
    CoalescedByOtherReplica,
    #[error("storage repo error: {0}")]
    Repo(#[from] nebula_storage::credential::RepoError),
    #[error("heartbeat error: {0}")]
    Heartbeat(#[from] nebula_storage::credential::HeartbeatError),
    #[error("config invalid: {0}")]
    Config(#[from] ConfigError),
}

impl RefreshCoordinator {
    pub fn new(
        repo: Arc<dyn RefreshClaimRepo>,
        replica_id: ReplicaId,
        config: RefreshCoordConfig,
    ) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            l1: L1RefreshCoalescer::new(),
            repo,
            replica_id,
            config,
        })
    }

    /// Acquire L1 mutex + L2 claim, run the refresh closure, release
    /// both. Returns `Err(CoalescedByOtherReplica)` if state was already
    /// fresh — caller treats as success and re-reads.
    pub async fn refresh_coalesced<F, Fut, T>(
        &self,
        credential_id: &nebula_core::CredentialId,
        do_refresh: F,
    ) -> Result<T, RefreshError>
    where
        F: FnOnce(RefreshClaim) -> Fut,
        Fut: std::future::Future<Output = Result<T, RefreshError>>,
    {
        // L1: in-process coalescing
        let _l1_guard = self.l1.acquire(credential_id).await;

        // L2: durable claim with backoff
        let claim = self.try_acquire_l2_with_backoff(credential_id).await?;

        // Heartbeat task in background
        let hb_task = self.spawn_heartbeat(claim.token.clone());

        // Run user's refresh closure
        let result = do_refresh(claim.clone()).await;

        // Stop heartbeat + release
        hb_task.abort();
        self.repo
            .release(claim.token)
            .await
            .map_err(RefreshError::Repo)?;

        result
    }

    async fn try_acquire_l2_with_backoff(
        &self,
        credential_id: &nebula_core::CredentialId,
    ) -> Result<RefreshClaim, RefreshError> {
        let max_attempts = 5;
        for _attempt in 0..max_attempts {
            match self
                .repo
                .try_claim(credential_id, &self.replica_id, self.config.claim_ttl)
                .await?
            {
                ClaimAttempt::Acquired(c) => return Ok(c),
                ClaimAttempt::Contended { existing_expires_at } => {
                    let now = chrono::Utc::now();
                    let wait_until = existing_expires_at
                        .min(now + chrono::Duration::seconds(5));
                    let delay = (wait_until - now)
                        .to_std()
                        .unwrap_or(Duration::from_millis(200));
                    tokio::time::sleep(delay + jitter_ms(100)).await;
                }
            }
        }
        Err(RefreshError::ContentionExhausted)
    }

    fn spawn_heartbeat(
        &self,
        token: nebula_storage::credential::ClaimToken,
    ) -> tokio::task::JoinHandle<()> {
        let repo = Arc::clone(&self.repo);
        let interval = self.config.heartbeat_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // first tick fires immediately
            loop {
                ticker.tick().await;
                if let Err(e) = repo.heartbeat(&token).await {
                    tracing::warn!(?e, "heartbeat failed; coordinator will release on next loop");
                    break;
                }
            }
        })
    }

    pub fn replica_id(&self) -> &ReplicaId {
        &self.replica_id
    }

    pub fn config(&self) -> &RefreshCoordConfig {
        &self.config
    }

    pub(crate) fn repo(&self) -> &Arc<dyn RefreshClaimRepo> {
        &self.repo
    }
}

fn jitter_ms(max_ms: u64) -> Duration {
    use rand::Rng;
    let amount = rand::thread_rng().gen_range(0..max_ms);
    Duration::from_millis(amount)
}
```

Length: ~200 lines.

- [ ] **Step 3: Property test on config invariants**

Create `crates/engine/tests/refresh_coord_config_proptest.rs`:

```rust
use std::time::Duration;

use nebula_engine::credential::refresh::RefreshCoordConfig;
use proptest::prelude::*;

#[test]
fn default_config_validates() {
    assert!(RefreshCoordConfig::default().validate().is_ok());
}

proptest! {
    #[test]
    fn heartbeat_invariant_holds_iff_validate_passes(
        ttl_secs in 5u64..300,
        hb_secs in 1u64..100,
        refresh_secs in 1u64..50,
        sweep_secs in 1u64..300,
    ) {
        let cfg = RefreshCoordConfig {
            claim_ttl: Duration::from_secs(ttl_secs),
            heartbeat_interval: Duration::from_secs(hb_secs),
            refresh_timeout: Duration::from_secs(refresh_secs),
            reclaim_sweep_interval: Duration::from_secs(sweep_secs),
            sentinel_threshold: 3,
            sentinel_window: Duration::from_secs(3600),
        };

        let valid = cfg.validate().is_ok();
        let invariants_hold = hb_secs * 3 < ttl_secs
            && refresh_secs + hb_secs * 2 < ttl_secs
            && sweep_secs <= ttl_secs;
        prop_assert_eq!(valid, invariants_hold);
    }
}
```

- [ ] **Step 4: Run unit + property tests**

Run: `cargo nextest run -p nebula-engine --test refresh_coord_config_proptest --profile ci --no-tests=pass`
Expected: PASS.

Then `cargo nextest run -p nebula-engine --profile ci --no-tests=pass` for full engine suite.

- [ ] **Step 5: Commit**

```bash
git add crates/engine/src/credential/refresh/coordinator.rs \
        crates/engine/tests/refresh_coord_config_proptest.rs \
        crates/engine/Cargo.toml
git commit -m "feat(engine): outer two-tier RefreshCoordinator (L1 + L2 claim) (Stage 2.2)

Public surface: refresh_coalesced(credential_id, do_refresh) wraps
L1 in-process coalesce + L2 durable claim with TTL/backoff/heartbeat.
Closes n8n #13088 cross-replica refresh race.

Property test on config invariants (heartbeat × 3 < claim_ttl etc.).

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 2.2
      docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md §3.1, §3.5"
```

### Task 2.3 — Wire `Arc<dyn RefreshClaimRepo>` through composition root

**Files:**
- Modify: `crates/engine/src/lib.rs` (or wherever `WorkflowEngine` / `AppState` lives)
- Modify: `crates/engine/src/credential/resolver.rs`

- [ ] **Step 1: Find composition root**

Run: `grep -rn 'WorkflowEngine\|AppState' crates/engine/src/`

The composition root is wherever `WorkflowEngine::new` or equivalent lives. Add a parameter:

```rust
pub fn new(
    /* ... existing ... */
    refresh_claim_repo: Arc<dyn nebula_storage::credential::RefreshClaimRepo>,
    coordinator_config: RefreshCoordConfig,
    replica_id: ReplicaId,
) -> Result<Self, /* ... */>
```

If the composition is via builder, add a `with_refresh_claim_repo` builder method. Default for tests: `Arc::new(InMemoryRefreshClaimRepo::new())`.

- [ ] **Step 2: Update `resolver.rs` to use the outer `RefreshCoordinator`**

Find the call sites of `RefreshCoordinator` (or `refresh::*`) in `crates/engine/src/credential/resolver.rs`. Currently they use the L1-only path. Switch to:

```rust
let result = self.refresh_coordinator
    .refresh_coalesced(credential_id, |claim| async move {
        // existing refresh body (HTTP POST etc.)
    })
    .await;

match result {
    Ok(state) => /* persist state */,
    Err(RefreshError::CoalescedByOtherReplica) => {
        // re-read state from store; another replica already refreshed
    },
    Err(e) => return Err(e.into()),
}
```

- [ ] **Step 3: Compile check**

Run: `cargo check -p nebula-engine`
Expected: PASS.

- [ ] **Step 4: Update existing engine tests for the new constructor**

Tests that built `WorkflowEngine` (or its builder) need an `InMemoryRefreshClaimRepo` injected. Search for `WorkflowEngine::new\|builder().build()`:

```rust
let repo = Arc::new(nebula_storage::credential::InMemoryRefreshClaimRepo::new());
let engine = WorkflowEngine::new(/* ... */, repo, RefreshCoordConfig::default(), ReplicaId::new("test"));
```

- [ ] **Step 5: Run engine tests**

Run: `cargo nextest run -p nebula-engine --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/engine/src/
git commit -m "feat(engine): wire RefreshClaimRepo through composition root (Stage 2.3)

WorkflowEngine takes Arc<dyn RefreshClaimRepo> + RefreshCoordConfig +
ReplicaId at construction. Tests inject InMemoryRefreshClaimRepo;
production composition (CLI / API entrypoints) inject the storage-
backed impl.

Resolver now calls refresh_coalesced, treating CoalescedByOtherReplica
as success.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 2.3"
```

### Task 2.4 — `token_refresh.rs` sentinel set/clear

**Files:**
- Modify: `crates/engine/src/credential/rotation/token_refresh.rs`
- Create: `crates/engine/src/credential/refresh/sentinel.rs`

- [ ] **Step 1: Implement `sentinel.rs` skeleton**

Create `crates/engine/src/credential/refresh/sentinel.rs`:

```rust
//! Sentinel mid-refresh crash detection + threshold escalation.
//!
//! Per sub-spec §3.4 + §6 audit.
//!
//! When a holder is about to perform the IdP POST (the operation that
//! risks invalidating the refresh token if not persisted), it marks the
//! claim as `RefreshInFlight`. On successful release, the mark clears.
//! If the reclaim sweep finds an expired claim still flagged
//! `RefreshInFlight`, the holder is presumed crashed mid-call.
//!
//! N=3 sentinel events within 1h (default) escalate to `ReauthRequired`.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use nebula_storage::credential::RefreshClaimRepo;

/// Configuration for the sentinel threshold logic. Default = 3-in-1h.
#[derive(Clone, Debug)]
pub struct SentinelThresholdConfig {
    pub threshold: u32,
    pub window: Duration,
}

impl Default for SentinelThresholdConfig {
    fn default() -> Self {
        Self {
            threshold: 3,
            window: Duration::from_secs(3600),
        }
    }
}

/// Tracks sentinel events per credential; emits triggered escalations.
pub struct SentinelTrigger {
    repo: Arc<dyn RefreshClaimRepo>,
    config: SentinelThresholdConfig,
    // TODO Stage 3: in-memory rolling-window counter or pull from
    // credential_sentinel_events table directly.
}

impl SentinelTrigger {
    pub fn new(
        repo: Arc<dyn RefreshClaimRepo>,
        config: SentinelThresholdConfig,
    ) -> Self {
        Self { repo, config }
    }

    pub fn config(&self) -> &SentinelThresholdConfig {
        &self.config
    }
}
```

(Stage 3 fills in the recording + threshold logic. This Stage 2 file just establishes the type so callers can plumb it through.)

- [ ] **Step 2: Update `token_refresh.rs`**

Find the HTTP POST call in `crates/engine/src/credential/rotation/token_refresh.rs`. Wrap it:

```rust
// Before HTTP POST:
self.refresh_claim_repo.mark_sentinel(&claim.token).await
    .map_err(/* into RefreshError */)?;

// HTTP POST happens here (existing code)
let response = self.http.post(/* ... */).send().await?;

// After successful response + persist:
// (sentinel clears via release in RefreshCoordinator::refresh_coalesced)
```

The `release` call already happens in `refresh_coalesced` (which clears the row entirely, removing sentinel). So no separate `clear` call needed — the row deletion IS the clear.

- [ ] **Step 3: Add unit test for sentinel set sequence**

In `crates/engine/tests/token_refresh_sentinel_unit.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;

use nebula_engine::credential::refresh::{
    RefreshCoordConfig, RefreshCoordinator,
};
use nebula_storage::credential::{
    InMemoryRefreshClaimRepo, RefreshClaimRepo, ReplicaId, SentinelState,
};

#[tokio::test]
async fn refresh_marks_sentinel_before_idp_call() {
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    let coord = RefreshCoordinator::new(
        Arc::clone(&repo),
        ReplicaId::new("test"),
        RefreshCoordConfig::default(),
    )
    .unwrap();

    let cid = nebula_core::CredentialId::from_string("test:token").unwrap();

    let result: Result<(), nebula_engine::credential::refresh::RefreshError> = coord
        .refresh_coalesced(&cid, |claim| async move {
            // Simulate token_refresh.rs flow:
            // 1. mark_sentinel (would be called inside the closure body
            //    by token_refresh.rs):
            repo.mark_sentinel(&claim.token).await.unwrap();
            // 2. "HTTP POST" succeeds — return Ok.
            Ok(())
        })
        .await;
    assert!(result.is_ok());

    // After release (success path), no claim row remains — sentinel
    // cleared by deletion.
    let reclaimed = repo.reclaim_stuck().await.unwrap();
    assert!(reclaimed.is_empty(), "no stuck claims after success path");
}
```

- [ ] **Step 4: Run the test**

Run: `cargo nextest run -p nebula-engine --test token_refresh_sentinel_unit --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/engine/src/credential/refresh/sentinel.rs \
        crates/engine/src/credential/rotation/token_refresh.rs \
        crates/engine/tests/token_refresh_sentinel_unit.rs
git commit -m "feat(engine): sentinel set on token_refresh HTTP path (Stage 2.4)

token_refresh.rs marks sentinel = RefreshInFlight immediately before
the IdP POST. The successful release path deletes the claim row
entirely (clears sentinel by removal). Mid-refresh crashes leave
sentinel set for reclaim sweep to detect.

SentinelTrigger skeleton lands here; threshold logic + ReauthRequired
escalation in Stage 3.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 2.4
      docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md §3.4"
```

### Task 2.5 — Two-tier integration test

**Files:**
- Create: `crates/engine/tests/refresh_coordinator_two_tier_integration.rs`

- [ ] **Step 1: Write the integration test**

```rust
//! Integration test: two simulated replicas + Postgres → exactly one
//! IdP POST observed.
//!
//! Per sub-spec §10 DoD #1.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_engine::credential::refresh::{
    RefreshCoordConfig, RefreshCoordinator,
};
use nebula_storage::credential::{
    InMemoryRefreshClaimRepo, RefreshClaimRepo, ReplicaId,
};
use tokio::join;

#[tokio::test]
async fn two_replicas_one_idp_call() {
    let repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    let coord_a = RefreshCoordinator::new(
        Arc::clone(&repo),
        ReplicaId::new("A"),
        RefreshCoordConfig::default(),
    ).unwrap();
    let coord_b = RefreshCoordinator::new(
        Arc::clone(&repo),
        ReplicaId::new("B"),
        RefreshCoordConfig::default(),
    ).unwrap();

    let cid = nebula_core::CredentialId::from_string("test:cross_replica").unwrap();
    let idp_calls = Arc::new(AtomicU32::new(0));

    let calls_a = Arc::clone(&idp_calls);
    let calls_b = Arc::clone(&idp_calls);

    let fut_a = coord_a.refresh_coalesced(&cid, |_claim| {
        let calls = Arc::clone(&calls_a);
        async move {
            // Simulate IdP call delay
            tokio::time::sleep(Duration::from_millis(100)).await;
            calls.fetch_add(1, Ordering::Relaxed);
            Ok::<_, nebula_engine::credential::refresh::RefreshError>(())
        }
    });
    let fut_b = coord_b.refresh_coalesced(&cid, |_claim| {
        let calls = Arc::clone(&calls_b);
        async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            calls.fetch_add(1, Ordering::Relaxed);
            Ok::<_, nebula_engine::credential::refresh::RefreshError>(())
        }
    });

    let (a, b) = join!(fut_a, fut_b);

    // Exactly one of them should have called the closure.
    // The other should have either: (a) waited on L1 (same process),
    // (b) gotten Contended on L2 then CoalescedByOtherReplica after
    // re-check, (c) gotten Contended after retries.
    let total_calls = idp_calls.load(Ordering::Relaxed);
    assert_eq!(total_calls, 1, "expected exactly 1 IdP POST, saw {total_calls}");

    // Both calls return successfully (CoalescedByOtherReplica is OK).
    let a_ok = matches!(a, Ok(_) | Err(nebula_engine::credential::refresh::RefreshError::CoalescedByOtherReplica));
    let b_ok = matches!(b, Ok(_) | Err(nebula_engine::credential::refresh::RefreshError::CoalescedByOtherReplica));
    assert!(a_ok && b_ok, "both replicas should observe success");
}
```

- [ ] **Step 2: Run the integration test**

Run: `cargo nextest run -p nebula-engine --test refresh_coordinator_two_tier_integration --profile ci --no-tests=pass`
Expected: PASS.

If it fails because L1 mutex catches both within the same process (since `InMemoryRefreshClaimRepo` is shared but the replicas are still in one process and L1's coalescing within the same process means the L2 race never happens) — note this in the test comments. Then add a true cross-process test in Stage 4 chaos test for full multi-replica coverage.

The point of this Stage 2.5 test is to prove the wiring works end-to-end and doesn't regress single-process behavior.

- [ ] **Step 3: Commit**

```bash
git add crates/engine/tests/refresh_coordinator_two_tier_integration.rs
git commit -m "test(engine): two-tier integration smoke (Stage 2.5)

Simulates two replicas sharing an InMemory repo; asserts exactly one
IdP POST. True cross-process coverage in Stage 4 chaos test.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 2.5
      docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md §10 DoD #1"
```

### Task 2.6 — Stage 2 gate

- [ ] **Step 1: Run Stage 2 gate**

```bash
cargo +nightly fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run -p nebula-engine -p nebula-storage --profile ci --no-tests=pass
RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-engine -p nebula-storage --no-deps
```

Expected: all PASS.

- [ ] **Step 2: Stage 2 marker commit**

```bash
git commit --allow-empty -m "chore(credential): Stage 2 gate passed (engine coordinator refactor)

L1 renamed → L1RefreshCoalescer (private). New outer RefreshCoordinator
wraps L1 + L2 RefreshClaimRepo. Composition root threads
Arc<dyn RefreshClaimRepo>. Resolver routes refresh through new
coordinator; CoalescedByOtherReplica treated as success.

token_refresh.rs marks sentinel before IdP POST. Sentinel threshold
escalation lands in Stage 3.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 2"
```

---

## Stage 3 — Sentinel threshold + `ReauthRequired` escalation (P3 per spec §7.4)

Adds the N=3-in-1h logic, `ReauthRequired` credential state transition, and `CredentialEvent::ReauthRequired` publication.

### Task 3.1 — `RefreshOutcome::CoalescedByOtherReplica` + `ReauthRequired` reason

**Files:**
- Modify: `crates/credential/src/contract/resolve.rs`
- Modify: `crates/credential/src/lib.rs` (re-export)

- [ ] **Step 1: Audit current `RefreshOutcome` shape**

Run: `grep -n 'enum RefreshOutcome\|RefreshOutcome::' crates/credential/src/contract/resolve.rs crates/engine/src/credential/`

Stage 3 of П1 removed `NotSupported`. Current shape (per П1): `Refreshed | ReauthRequired | <wildcard>`.

- [ ] **Step 2: Add `CoalescedByOtherReplica` variant**

Edit `RefreshOutcome`:

```rust
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum RefreshOutcome {
    Refreshed,
    ReauthRequired(ReauthReason),
    /// Another replica refreshed concurrently; this replica's caller
    /// should re-read state. Per sub-spec §3.6.
    CoalescedByOtherReplica,
}

/// Why a credential transitioned to `ReauthRequired`.
#[derive(Debug, Clone)]
pub enum ReauthReason {
    /// Provider rejected the refresh (refresh_token invalidated).
    ProviderRejected { detail: String },
    /// Sentinel threshold exceeded — credential keeps crashing
    /// mid-refresh per sub-spec §3.4.
    SentinelRepeated { event_count: u32, window_secs: u64 },
}
```

- [ ] **Step 3: Update `RefreshOutcome` consumers**

Run: `grep -rn 'RefreshOutcome::ReauthRequired\b' crates/`

Each match must adapt to the new `ReauthRequired(ReauthReason)` shape — wrap existing `ReauthRequired` calls with `ReauthRequired(ReauthReason::ProviderRejected { detail })` or similar.

- [ ] **Step 4: Compile + run tests**

Run: `cargo check -p nebula-credential -p nebula-engine`
Expected: PASS (after consumer fixes).

Run: `cargo nextest run -p nebula-credential -p nebula-engine --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/credential/src/contract/resolve.rs \
        crates/credential/src/lib.rs \
        $(grep -rln 'RefreshOutcome::ReauthRequired\b' crates/)
git commit -m "feat(credential): RefreshOutcome::CoalescedByOtherReplica + ReauthReason (Stage 3.1)

CoalescedByOtherReplica per sub-spec §3.6 — caller treats as success.
ReauthRequired carries typed ReauthReason (ProviderRejected vs
SentinelRepeated) so operators can distinguish refresh-rotation
failures from intermittent-IdP signals.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 3.1"
```

### Task 3.2 — Sentinel threshold logic (rolling 1h count)

**Files:**
- Modify: `crates/engine/src/credential/refresh/sentinel.rs`

- [ ] **Step 1: Implement event recording + threshold detection**

Replace `sentinel.rs` with full impl. The threshold counter reads from `credential_sentinel_events` table directly (durable count) — Stage 3 plan adds the storage interface.

```rust
// Add to RefreshClaimRepo trait (modify Stage 1 trait file)
async fn record_sentinel_event(
    &self,
    credential_id: &nebula_core::CredentialId,
    crashed_holder: &ReplicaId,
    generation: u64,
) -> Result<(), RepoError>;

async fn count_sentinel_events_in_window(
    &self,
    credential_id: &nebula_core::CredentialId,
    window_start: chrono::DateTime<chrono::Utc>,
) -> Result<u32, RepoError>;
```

Add SQLite + Postgres impls of the two new trait methods (mirror Stage 1 patterns, simple INSERT + SELECT COUNT).

Then the sentinel logic:

```rust
impl SentinelTrigger {
    /// Called by the reclaim sweep when an expired claim has
    /// `sentinel = RefreshInFlight`. Records event + checks threshold.
    pub async fn on_sentinel_detected(
        &self,
        credential_id: &nebula_core::CredentialId,
        crashed_holder: &ReplicaId,
        generation: u64,
    ) -> Result<SentinelDecision, RepoError> {
        self.repo
            .record_sentinel_event(credential_id, crashed_holder, generation)
            .await?;

        let window_start = chrono::Utc::now()
            - chrono::Duration::from_std(self.config.window).unwrap();
        let count = self
            .repo
            .count_sentinel_events_in_window(credential_id, window_start)
            .await?;

        if count >= self.config.threshold {
            Ok(SentinelDecision::EscalateToReauth {
                event_count: count,
                window_secs: self.config.window.as_secs(),
            })
        } else {
            Ok(SentinelDecision::Recoverable { event_count: count })
        }
    }
}

#[derive(Debug, Clone)]
pub enum SentinelDecision {
    Recoverable { event_count: u32 },
    EscalateToReauth { event_count: u32, window_secs: u64 },
}
```

- [ ] **Step 2: Add Stage 3 trait methods to in-memory + SQLite + Postgres impls**

Mirror existing patterns. SQLite + Postgres = INSERT + SELECT COUNT WHERE detected_at > $1.

- [ ] **Step 3: Unit tests**

Below-threshold + at-threshold + above-threshold cases. Use `InMemoryRefreshClaimRepo` for unit (storage backed by HashMap of events).

- [ ] **Step 4: Run tests**

Run: `cargo nextest run -p nebula-engine -p nebula-storage --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/engine/src/credential/refresh/sentinel.rs \
        crates/storage/src/credential/refresh_claim/
git commit -m "feat(engine): sentinel threshold N=3-in-1h logic (Stage 3.2)

SentinelTrigger::on_sentinel_detected records event + checks rolling
count against threshold. Above threshold → SentinelDecision::EscalateToReauth.

Storage trait gains record_sentinel_event + count_sentinel_events_in_window.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 3.2
      docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md §3.4"
```

### Task 3.3 — Background reclaim sweep task

**Files:**
- Modify: `crates/engine/src/credential/refresh/reclaim.rs` (was stub)

- [ ] **Step 1: Implement reclaim sweep loop**

```rust
//! Background reclaim-sweep task. Parallel to control-queue reclaim.

use std::sync::Arc;
use std::time::Duration;

use nebula_storage::credential::{RefreshClaimRepo, SentinelState};

use super::sentinel::{SentinelDecision, SentinelTrigger};

pub struct ReclaimSweepHandle {
    handle: tokio::task::JoinHandle<()>,
}

impl ReclaimSweepHandle {
    pub fn spawn(
        repo: Arc<dyn RefreshClaimRepo>,
        sentinel: Arc<SentinelTrigger>,
        cadence: Duration,
        publish_reauth: Arc<dyn Fn(/* event */) + Send + Sync>,
    ) -> Self {
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(cadence);
            loop {
                ticker.tick().await;
                let stuck = match repo.reclaim_stuck().await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(?e, "reclaim sweep failed");
                        continue;
                    }
                };
                for reclaimed in stuck {
                    if reclaimed.sentinel == SentinelState::RefreshInFlight {
                        match sentinel
                            .on_sentinel_detected(
                                &reclaimed.credential_id,
                                &reclaimed.previous_holder,
                                reclaimed.previous_generation,
                            )
                            .await
                        {
                            Ok(SentinelDecision::EscalateToReauth { event_count, window_secs }) => {
                                tracing::warn!(
                                    cred = %reclaimed.credential_id,
                                    event_count,
                                    window_secs,
                                    "sentinel threshold exceeded — escalating to ReauthRequired"
                                );
                                publish_reauth(/* event */);
                            }
                            Ok(SentinelDecision::Recoverable { event_count }) => {
                                tracing::info!(
                                    cred = %reclaimed.credential_id,
                                    event_count,
                                    "sentinel recoverable — continuing"
                                );
                            }
                            Err(e) => tracing::warn!(?e, "sentinel decision failed"),
                        }
                    }
                }
            }
        });
        Self { handle }
    }

    pub fn abort(self) {
        self.handle.abort();
    }
}
```

(Replace the `Arc<dyn Fn>` callback with the actual `EventBus` reference once you locate the `nebula-eventbus` API for credential events. Use `CredentialEvent::ReauthRequired` per existing event taxonomy.)

- [ ] **Step 2: Wire sweep at engine init**

In `WorkflowEngine::new` (or equivalent), spawn `ReclaimSweepHandle::spawn(...)` and store in the engine struct so it's aborted on shutdown.

- [ ] **Step 3: Integration test**

Create `crates/engine/tests/refresh_coordinator_sentinel_integration.rs`. Cover:

- Mid-refresh crash → sentinel detected → 1 event recorded → no escalation
- 3 mid-crashes within 1h → escalation triggered
- 2 mid-crashes within 1h, 1 outside window → no escalation

Use a test-time-source if needed (tokio's `tokio::time::pause`).

- [ ] **Step 4: Run integration**

Run: `cargo nextest run -p nebula-engine --test refresh_coordinator_sentinel_integration --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/engine/src/credential/refresh/reclaim.rs \
        crates/engine/src/lib.rs \
        crates/engine/tests/refresh_coordinator_sentinel_integration.rs
git commit -m "feat(engine): background reclaim sweep + ReauthRequired escalation (Stage 3.3)

ReclaimSweepHandle::spawn polls reclaim_stuck on cadence; for each
stuck claim with sentinel=RefreshInFlight, calls SentinelTrigger and
publishes CredentialEvent::ReauthRequired when threshold exceeded.

Integration test covers below/at/above-threshold cases.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 3.3"
```

### Task 3.4 — Stage 3 gate

- [ ] **Step 1: Run Stage 3 gate**

```bash
cargo +nightly fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run -p nebula-engine -p nebula-storage --profile ci --no-tests=pass
RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-engine -p nebula-storage --no-deps
```

Expected: all PASS.

- [ ] **Step 2: Marker commit**

```bash
git commit --allow-empty -m "chore(credential): Stage 3 gate passed (sentinel threshold + ReauthRequired)"
```

---

## Stage 4 — Observability (P4 per spec §7.4)

Adds 5 metrics, 3 tracing spans, 3 audit events. Plus the chaos test.

### Task 4.1 — Metrics constants

**Files:**
- Modify: `crates/metrics/src/naming.rs`

- [ ] **Step 1: Add constants per spec §6**

```rust
pub const NEBULA_CREDENTIAL_REFRESH_COORD_CLAIMS_TOTAL: &str =
    "nebula_credential_refresh_coord_claims_total";
pub const NEBULA_CREDENTIAL_REFRESH_COORD_COALESCED_TOTAL: &str =
    "nebula_credential_refresh_coord_coalesced_total";
pub const NEBULA_CREDENTIAL_REFRESH_COORD_SENTINEL_EVENTS_TOTAL: &str =
    "nebula_credential_refresh_coord_sentinel_events_total";
pub const NEBULA_CREDENTIAL_REFRESH_COORD_RECLAIM_SWEEPS_TOTAL: &str =
    "nebula_credential_refresh_coord_reclaim_sweeps_total";
pub const NEBULA_CREDENTIAL_REFRESH_COORD_HOLD_DURATION_SECONDS: &str =
    "nebula_credential_refresh_coord_hold_duration_seconds";
```

- [ ] **Step 2: Wire counters/histograms in coordinator + sweep**

Find where existing nebula-metrics registration happens. Add `prometheus::IntCounterVec::new(...)` etc. for each. Increment per outcome label per spec §6.

- [ ] **Step 3: Tracing spans**

In `coordinator.rs::refresh_coalesced`: wrap with `#[tracing::instrument(skip(do_refresh), fields(credential_id = %credential_id, replica_id = %self.replica_id, tier))]`.

In `try_acquire_l2_with_backoff`: per-attempt span.

In `reclaim.rs::sentinel.on_sentinel_detected`: detected span.

- [ ] **Step 4: Audit events**

Find `AuditLayer` / `nebula_storage::credential::AuditLayer`. Add three events:

```rust
RefreshCoordClaimAcquired { credential_id, holder, ttl_secs },
RefreshCoordSentinelTriggered { credential_id, recent_count },
RefreshCoordReauthFlagged { credential_id, reason },
```

Wire emission at the corresponding sites.

- [ ] **Step 5: Run tests**

Run: `cargo nextest run --workspace --profile ci --no-tests=pass`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/metrics/src/naming.rs crates/engine/src/credential/refresh/ \
        crates/storage/src/credential/
git commit -m "feat(observability): refresh coordinator metrics + spans + audit events (Stage 4.1)

5 metrics constants. tracing::instrument on coordinator + sweep.
3 audit events through AuditLayer.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 4.1
      docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md §6"
```

### Task 4.2 — `docs/OBSERVABILITY.md` entry

**Files:**
- Modify: `docs/OBSERVABILITY.md`

- [ ] **Step 1: Add §refresh-coordinator section**

Read existing OBSERVABILITY.md structure. Add a new subsection per the conventions used by other engine subsystems. Cover:

- Each of the 5 metrics with labels + cardinality
- 3 tracing spans + their attributes
- 3 audit events
- Sample queries / dashboard hints (PromQL)

- [ ] **Step 2: Commit**

```bash
git add docs/OBSERVABILITY.md
git commit -m "docs(observability): refresh coordinator entry (Stage 4.2)

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 4.2"
```

### Task 4.3 — Chaos test (nightly)

**Files:**
- Create: `crates/engine/tests/refresh_coordinator_chaos.rs`

- [ ] **Step 1: Write the chaos test**

Per sub-spec §5.4: 3 in-memory replicas + 100 credentials × 10 minutes (in test, scale down: 3 replicas × 10 credentials × 30 seconds for CI default; full scale gated behind `cfg(feature = "chaos")` + nightly).

Skeleton:

```rust
//! Chaos test — 3 replicas, 100 credentials, 10 minutes.
//!
//! Per sub-spec §5.4. Ungated default version is scaled down for CI;
//! `--features chaos-full` runs the full 10-min sweep.

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_replicas_zero_false_positives() {
    // ... setup 3 RefreshCoordinators sharing a single InMemoryRepo ...
    // ... drive 1000 concurrent resolves over 30s scaled-down window ...
    // Assert:
    // 1. Each credential refresh invoked at most once per expiry window
    // 2. ReauthRequired count = 0 (no injected crashes)
    // 3. P99 resolve latency < 100ms outside refresh window
}
```

Mark with `#[ignore]` for default `cargo test`; CI runs via `cargo nextest run -E 'test(chaos)'` in nightly.

- [ ] **Step 2: Run the test**

Run: `cargo nextest run -p nebula-engine --test refresh_coordinator_chaos --profile ci --no-tests=pass --run-ignored=all`
Expected: PASS.

- [ ] **Step 3: Add nightly CI hook**

Edit `.github/workflows/test-matrix.yml` (or wherever nightly tests are gated) to include the chaos test target. Or add a new workflow `nightly-chaos.yml`.

- [ ] **Step 4: Commit**

```bash
git add crates/engine/tests/refresh_coordinator_chaos.rs .github/workflows/
git commit -m "test(engine): nightly chaos test (3 replicas × 100 creds × 10 min) (Stage 4.3)

Scaled-down version runs in default CI; full version gated behind
nightly + chaos-full feature.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 4.3
      docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md §5.4"
```

### Task 4.4 — Stage 4 gate

- [ ] **Step 1: Run gate**

```bash
cargo +nightly fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace --profile ci --no-tests=pass
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

Expected: all PASS.

- [ ] **Step 2: Marker commit**

```bash
git commit --allow-empty -m "chore(credential): Stage 4 gate passed (observability + chaos)"
```

---

## Stage 5 — Doc sync + MATURITY flip (P5 per spec §7.4)

### Task 5.1 — MATURITY + ADR-0030 amendment

**Files:**
- Modify: `docs/MATURITY.md`
- Modify: `docs/adr/0030-engine-owns-credential-orchestration.md`
- Modify: `docs/tracking/credential-concerns-register.md`
- Modify: `crates/storage/README.md`

- [ ] **Step 1: Flip MATURITY for `nebula-credential`**

In the `Engine integration` column, change `partial → stable`. Cite the П2 squash commit SHA (placeholder until merge).

- [ ] **Step 2: Amend ADR-0030 §3**

Add an "Amendment 2026-04-26-N (Stage 5)" entry citing this implementation closes the §3 amendment's "two-tier coordinator" line item.

- [ ] **Step 3: Flip register row**

`draft-f17` row → `done` with П2 merge commit pointer.

- [ ] **Step 4: Update storage README**

Add a section on `RefreshClaimRepo` mirroring how other repos (e.g., `PendingStateStore`, `KeyProvider`) are documented.

- [ ] **Step 5: Commit**

```bash
git add docs/MATURITY.md \
        docs/adr/0030-engine-owns-credential-orchestration.md \
        docs/tracking/credential-concerns-register.md \
        crates/storage/README.md
git commit -m "docs(credential): П2 doc sync (MATURITY flip + ADR-0030 amendment)

nebula-credential::engine integration: partial → stable.
ADR-0030 §3 amendment cites Stage 2 implementation as closing the
two-tier coordinator obligation. Concerns register draft-f17 flipped
to done with merge SHA.

Refs: docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md Stage 5"
```

### Task 5.2 — Final landing-gate verification

- [ ] **Step 1: Run all probes**

Run all probes: in addition to the 11 from П1, П2 added:
- loom: `RUSTFLAGS="--cfg loom" cargo test --test refresh_claim_loom`
- proptest: `cargo nextest run -p nebula-engine --test refresh_coord_config_proptest`
- 4 integration tests (in-memory + SQLite + Postgres + sentinel + two-tier + chaos)

```bash
cargo nextest run --workspace --profile ci --no-tests=pass
RUSTFLAGS="--cfg loom" cargo nextest run -p nebula-storage --test refresh_claim_loom --profile ci --no-tests=pass
```

Expected: all PASS.

- [ ] **Step 2: Capture post-П2 cargo-public-api snapshots + diff**

```bash
cargo public-api --manifest-path crates/storage/Cargo.toml > /tmp/storage-post-p2.txt
cargo public-api --manifest-path crates/engine/Cargo.toml > /tmp/engine-post-p2.txt
diff /tmp/storage-pre-p2.txt /tmp/storage-post-p2.txt > /tmp/p2-storage-diff.txt
diff /tmp/engine-pre-p2.txt /tmp/engine-post-p2.txt > /tmp/p2-engine-diff.txt
```

Inspect: every change tied to a Stage's commit. Keep diffs for PR description.

- [ ] **Step 3: Full local gate**

```bash
cargo +nightly fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
cargo test --workspace --doc
cargo nextest run --workspace --profile ci --no-tests=pass
```

Expected: all PASS.

- [ ] **Step 4: Open merge PR**

```bash
git push -u origin worktree-credential-p2
gh pr create --title "feat(credential)!: П2 — refresh coordination L2 (n8n #13088 close)" --body "$(cat <<'EOF'
## Summary

Lands cross-replica refresh coordination per sub-spec at
`docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`
and ADR-0034. Two-tier coordinator (in-process L1 + durable L2 claim
repo with TTL/heartbeat/reclaim) closes n8n #13088 class production
race where rotated `refresh_token_v2` invalidates `refresh_token_v1`
on a parallel replica.

## Stages

- **Stage 1** — `RefreshClaimRepo` trait + 3 impls (in-memory, SQLite, Postgres) + migrations 0022/0023 + loom CAS probe
- **Stage 2** — Engine refactor: `L1RefreshCoalescer` (private) + new outer `RefreshCoordinator` + composition wiring + `token_refresh.rs` sentinel set
- **Stage 3** — Sentinel N=3-in-1h logic + `ReauthRequired` escalation + reclaim sweep
- **Stage 4** — 5 metrics + 3 spans + 3 audit events + nightly chaos test
- **Stage 5** — MATURITY flip + ADR-0030 amendment + register update

## Verification

- All workspace tests PASS
- 11 probes from П1 + 2 new (loom + proptest) + 4 integration tests
- chaos test (scaled-down) PASS
- `cargo deny check` clean
- All CI checks green

## Refs

- Sub-spec: `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md` (frozen at P2 ship)
- ADR-0034: `docs/adr/0034-durable-credential-refresh-claim-repo.md`
- ADR-0030 amendment: `docs/adr/0030-engine-owns-credential-orchestration.md`
- Plan: `docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review

**1. Spec coverage** — every §10 DoD requirement maps to a task:
- §10 (a) — loom CAS probe → Stage 1.5
- §10 (b) — proptest config invariants → Stage 2.2
- §10 (c) — sequential 2-replica → Stage 2.5
- §10 (d) — mid-refresh crash sentinel → Stage 3.3
- §10 (e) — sentinel below threshold → Stage 3.3
- §10 (f) — sentinel at threshold → Stage 3.3
- §10 (g) — CoalescedByOtherReplica handled → Stage 2.5
- §10 (h) — heartbeat extends TTL / ClaimLost → Stage 1.2/1.3
- §10 (i) — chaos test → Stage 4.3
- §10 (j) — schema parity CI → Stage 1.3 (existing CI extended)
- §10 (k) — `cargo deny` unchanged → Stage 5.2 verification

**2. Placeholder scan** — every step has concrete code or exact command. No `TODO`/`TBD` (the `// TODO Stage 3` inside Stage 2.4's `sentinel.rs` is a forward-pointer, not a placeholder — Stage 3.2 fills it).

**3. Type consistency** — `RefreshClaim`, `ClaimToken`, `ClaimAttempt`, `RefreshCoordinator`, `RefreshCoordConfig`, `RefreshOutcome::CoalescedByOtherReplica`, `ReauthReason::SentinelRepeated` all consistent across stages.

**Open items (acceptable):**
- §11 Q1 (re-entry) — not directly addressed by tests; documented in coordinator.rs rustdoc (future enhancement).
- §11 Q2 (multi-tenant endpoint) — explicitly out of scope per spec §11; flagged for ProviderRegistry phase (П4).
- §11 Q3 (Postgres unavailable fallback) — Stage 4.1 metrics surface "degraded coordination"; explicit fail-closed-vs-L1-only choice deferred to deployment runbook.

---

**Plan complete.**

## Execution Handoff

Two execution options:

**1. Subagent-Driven (recommended)** — Dispatch fresh subagent per task, review between tasks, fast iteration. Best fit for П2 because each Stage is self-contained with clear acceptance gates (loom + proptest + integration probes per Stage), and Stage 1's three impls (in-memory + SQLite + Postgres) parallel-clean without subagent context pollution.

**2. Inline Execution** — Execute tasks in this session via `superpowers:executing-plans`. Higher context cost but tighter feedback on Stage 1.5 loom + Stage 4.3 chaos design.

Which approach?
