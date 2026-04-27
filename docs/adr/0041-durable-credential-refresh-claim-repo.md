---
id: 0041
title: durable-credential-refresh-claim-repo
status: proposed
date: 2026-04-26
supersedes: []
superseded_by: []
amends: [0030]
tags: [credential, engine, storage, refresh, oauth2, multi-replica, canon-13.2, canon-12.5, canon-4.5]
related:
  - docs/adr/0008-execution-control-queue-consumer.md
  - docs/adr/0017-control-queue-reclaim-policy.md
  - docs/adr/0028-cross-crate-credential-invariants.md
  - docs/adr/0029-storage-owns-credential-persistence.md
  - docs/adr/0030-engine-owns-credential-orchestration.md
  - docs/PRODUCT_CANON.md#125-secrets-and-auth
  - docs/PRODUCT_CANON.md#132-rotation-refresh-seam
  - docs/PRODUCT_CANON.md#45-operational-honesty--no-false-capabilities
  - docs/research/n8n-credential-pain-points.md
  - docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md
  - docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md
linear: []
---

# 0041. Durable credential refresh claim repository

> **Numbering note.** The associated sub-spec at
> `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`
> reserved slot **0034** in its frontmatter. That slot was subsequently
> consumed by `0034-schema-secret-value-credential-seam.md` (accepted
> 2026-04-22) before this ADR was drafted, so this decision lands at the
> next free number, **0041**. The sub-spec's intent is preserved
> verbatim — only the file name differs.

## Status

**Proposed** 2026-04-26. Will flip to `accepted` when the credential П2
implementation cascade lands its merge commit on `main`. This ADR codifies
the decision that the spec at
`docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`
already captures in detail; landing the spec's first storage migration is
the verifiable seam for the flip to accepted.

This ADR amends [ADR-0030 §3](./0030-engine-owns-credential-orchestration.md)
("`RefreshCoordinator` design note — concrete, not trait"). ADR-0030
explicitly anticipated this follow-up: *"If a future need arises for an
alternative coordination strategy (e.g., distributed refresh coalescing
across multiple nebula-engine replicas via a storage-backed lock), a new
ADR opens. That ADR does not relax `RefreshCoordinator`; it supersedes
this §3 decision and introduces a new primitive."* This is that ADR. The
in-process coordinator is preserved as an internal coalescer; the new
public `RefreshCoordinator` composes it with a durable storage-backed
claim repo.

## Context

[ADR-0030](./0030-engine-owns-credential-orchestration.md) §3 placed the
in-process `RefreshCoordinator` (LRU-bounded `parking_lot::Mutex` keyed by
`credential_id`) inside `nebula-engine::credential::refresh`. That
coordinator coalesces concurrent refreshes **within a single replica**. It
was sufficient when production deployments ran a single `nebula-engine`
process.

It does not coordinate across replicas. The race shape is concrete and
well-documented in the wild (see
[`docs/research/n8n-credential-pain-points.md`](../research/n8n-credential-pain-points.md)
§1, n8n issue #13088 — confirmed production race, unresolved upstream
since 2024):

```
Replica A: resolve(slack_cred) — near expiry
Replica A: L1.lock(slack_cred)
Replica A: POST {idp.token_endpoint} with refresh_token_v1
Replica A: ← access_token_v2 + refresh_token_v2 (v1 invalidated by IdP)
Replica A: storage::put(new_state)
Replica A: L1.unlock

Replica B (concurrent, different process):
Replica B: L1.lock(slack_cred)          ← DIFFERENT MUTEX
Replica B: read state — possibly still old (cache lag / read-your-write race)
Replica B: POST with refresh_token_v1 (now stale)
Replica B: ← IdP rejects: "refresh token already consumed"
Replica B: credential marked ReauthRequired — permanent failure
```

Most modern OAuth2 IdPs rotate refresh tokens on every refresh response
(Microsoft Azure AD, Google, Slack, Atlassian, …), which makes the
single-use property of the prior token a hard correctness constraint, not
a tunable. Any multi-replica deployment with a refresh-rotating IdP
exhibits this race with non-zero probability whenever two replicas hit a
near-expiry credential within the same window.

Canon obligations that bear on this:

- [§13.2 — Rotation / refresh seam](../PRODUCT_CANON.md#132-rotation-refresh-seam):
  the seam is `Credential::refresh()`; the orchestration enforcer is
  engine. ADR-0028 invariant 2.
- [§4.5 — Operational honesty](../PRODUCT_CANON.md#45-operational-honesty--no-false-capabilities):
  shipping multi-replica engine while leaving a known production race
  unmitigated would be a false capability.
- [§12.5 — Secrets and auth](../PRODUCT_CANON.md#125-secrets-and-auth):
  any new persisted state introduced for coordination MUST NOT carry
  credential material.

The full design is in
`docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`
(651 lines: sequence diagrams, schema DDL for both dialects, test plan,
parameter discipline, threshold logic, observability surface). This ADR
captures the load-bearing decisions; the spec is the implementation
reference.

## Decision

Adopt a **two-tier refresh coordinator**: the existing in-process
mutex-LRU becomes a private `L1RefreshCoalescer`; a new outer
`RefreshCoordinator` composes L1 with a durable, CAS-based
`RefreshClaimRepo` trait (L2) provided by `nebula-storage`. The shape
mirrors the control-queue claim pattern from
[ADR-0008](./0008-execution-control-queue-consumer.md) §1 and
[ADR-0017](./0017-control-queue-reclaim-policy.md): atomic CAS claim with
TTL, periodic heartbeat to extend the TTL, sweep-based reclaim when a
holder crashes.

### 1. Two-tier composition

```
engine::RefreshCoordinator
  ├── L1: L1RefreshCoalescer (private, in-process)
  │     LruCache<CredentialId, Arc<Mutex<()>>>  — same primitive as today
  │     Coalesces concurrent refreshes within one replica.
  └── L2: Arc<dyn RefreshClaimRepo> (durable, cross-replica)
        CAS INSERT/UPDATE on credential_refresh_claims.
        TTL 30s default; heartbeat 10s; reclaim sweep 30s.
```

Acquisition is L1 first, then L2: the L1 mutex prevents wasted L2
round-trips on intra-replica contention, while L2 provides cross-replica
correctness. The hot-path overhead (no contention, no near-expiry) is
unchanged from today — the coordinator short-circuits via `needs_refresh`
before any L2 call.

The public surface preserves the existing engine-side calling convention
(`refresh_coalesced(credential_id, do_refresh)`); engine call sites
upgrade transparently. The renamed inner `L1RefreshCoalescer` is
crate-private.

### 2. `RefreshClaimRepo` trait — storage-side seam

Lives in `nebula-storage::credential::refresh_claim`. Trait shape (full
shape + DTO definitions are in spec §3.2; abbreviated here):

```rust
#[async_trait]
pub trait RefreshClaimRepo: Send + Sync + 'static {
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError>;

    async fn heartbeat(&self, token: &ClaimToken) -> Result<(), HeartbeatError>;

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError>;

    async fn reclaim_stuck(&self) -> Result<Vec<CredentialId>, RepoError>;
}
```

`ClaimAttempt::Acquired(RefreshClaim)` carries an opaque
`ClaimToken { claim_id: Uuid, generation: u64 }`. The `generation` field
is bumped on every CAS write — heartbeats and releases that present a
stale generation fail with `HeartbeatError::ClaimLost`, preventing a
zombie holder from extending a TTL after another replica has reclaimed.

`ClaimAttempt::Contended { existing_expires_at }` returns the contender's
expiry so the loser can sleep until that wall-clock moment (plus jitter)
before the next attempt — bounded backoff without polling.

Three impls land alongside the trait:

- `InMemoryRefreshClaimRepo` — for unit tests + the desktop-mode
  no-storage fallback path.
- `SqliteRefreshClaimRepo` — desktop / single-replica deployments.
- `PgRefreshClaimRepo` — production / multi-replica deployments.

### 3. Schema overview — two new tables

**`credential_refresh_claims`** holds the active claim per credential
(one row per `credential_id`):

| Column              | Purpose                                                       |
| ------------------- | ------------------------------------------------------------- |
| `credential_id`     | PK. One claim per credential.                                 |
| `claim_id`          | UUID — identity of the active claim.                          |
| `generation`        | Bumped on every CAS write; rejects stale heartbeats.          |
| `holder_replica_id` | Diagnostic — which replica holds the claim.                   |
| `acquired_at`       | Timestamp the claim was first acquired in this generation.    |
| `expires_at`        | Acquired+TTL; reclaim sweep targets `expires_at < NOW()`.     |
| `sentinel`          | 0=normal, 1=refresh_in_flight (mid-refresh crash detection).  |

**`credential_sentinel_events`** records detected mid-refresh crashes
(append-only):

| Column            | Purpose                                                 |
| ----------------- | ------------------------------------------------------- |
| `credential_id`   | Which credential had a sentinel-flagged crash.          |
| `detected_at`     | When the reclaim sweep noticed the orphaned sentinel.   |
| `crashed_holder`  | The replica id whose claim expired with sentinel=1.     |
| `generation`      | The generation of the orphaned claim, for forensics.    |

A composite index on `(credential_id, detected_at)` supports the rolling
1-hour window query that drives threshold escalation (§5).

The full DDL — column types per dialect, indexes, dialect-specific
adjustments — lives in the storage migrations
(`crates/storage/migrations/{sqlite,postgres}/`). This ADR fixes the
high-level shape only.

**§12.5 binding.** Neither table carries credential material. Rows hold
identifiers, timestamps, holder strings, generation counters, and the
sentinel byte — nothing that re-encrypts at rest, nothing that needs the
KEK rotation path.

### 4. Mid-refresh crash sentinel

A claim holder marks `sentinel = 1` immediately before issuing the IdP
POST and clears it (with claim release) immediately after a successful
storage write of the refreshed state. If a crash happens between mark and
clear, the row stays `sentinel = 1` past `expires_at`. The reclaim sweep
detects this state and records a row in `credential_sentinel_events`.

A single sentinel event is **not** treated as a permanent fault. Slow IdP
responses combined with a too-tight TTL can produce false positives.
Permanent escalation requires N=3 sentinel events for the same
`credential_id` within a rolling 1-hour window (configurable). Below the
threshold, reclaim proceeds normally; at or above, the credential
transitions to `ReauthRequired` with reason `SentinelRepeated` and a
`CredentialEvent::ReauthRequired` is published.

### 5. Parameter discipline

`RefreshCoordConfig::validate()` enforces three invariants statically:

- `heartbeat_interval * 3 < claim_ttl` (a holder must heartbeat at least
  three times within its own TTL).
- `refresh_timeout + 2 * heartbeat_interval < claim_ttl` (a holder must
  finish or fail before its claim could plausibly be reclaimed).
- `reclaim_sweep_interval <= claim_ttl` (the sweep cadence cannot lag
  behind TTL or stuck claims accumulate).

The shipped defaults
(`claim_ttl = 30s`, `heartbeat = 10s`, `refresh_timeout = 8s`,
`sweep = 30s`, `sentinel_threshold = 3`, `sentinel_window = 1h`) satisfy
all three. A property test asserts `validate()` is consistent with the
arithmetic invariants over a wide parameter range; a unit test pins the
shipped defaults.

### 6. Background reclaim sweep

A new background task spawns from `engine::init()` alongside the existing
control-queue reclaim sweep (ADR-0017). It calls
`RefreshClaimRepo::reclaim_stuck` on the configured cadence and emits
metrics per outcome. Multi-replica safety follows the same pattern as
ADR-0017: every replica may sweep; CAS on the row guarantees at most one
reclaim per stuck row across all sweepers.

## Consequences

### Positive

- **Cross-replica refresh safety.** Two replicas resolving the same
  credential within the L2 TTL window result in exactly one IdP POST.
  Closes the n8n #13088-class race for any deployment running with the
  Postgres or SQLite claim repo wired in.
- **Mid-refresh crash detection.** A holder that crashes between
  `POST` and storage write surfaces as a sentinel event; repeated
  occurrences escalate to `ReauthRequired` with operator visibility,
  rather than leaving a credential in an undefined state until the next
  unsuccessful `resolve` call.
- **Pattern reuse.** The CAS+TTL+heartbeat+reclaim shape is the same
  one ADR-0008 / ADR-0017 use for control-queue claims. Reviewers
  tracing a refresh through `RefreshClaimRepo` follow the same shape as
  tracing a `Cancel` through `ControlQueueRepo`. No second coordination
  primitive to maintain mental models for.
- **Desktop-mode parity.** The SQLite impl is a strict translation of
  the Postgres CAS semantics (a single-process SQLite host has no
  concurrent writers, but the impl exists so the API contract is
  identical — the engine never branches on dialect).
- **`Engine integration` MATURITY can flip to `stable`.** ADR-0028
  invariant 5 names this row as the load-bearing operational-honesty
  signal; today it's `partial` precisely because of this gap. Closing
  the gap unblocks the flip.

### Negative / accepted costs

- **One extra storage round trip per actual refresh.** ~1-5 ms on
  Postgres, sub-millisecond on local SQLite. Refresh is already a
  seconds-scale operation (the IdP POST dominates); the claim
  round-trip is in the noise. Hot-path resolves with no near-expiry
  pay zero — `needs_refresh` short-circuits before the L2 call.
- **Two new tables to migrate, monitor, and drop on uninstall.** A
  schema-parity CI check (existing) covers dialect drift; the tables
  are bounded in size (one row per credential for claims; reclaimed
  rows for sentinel events accumulate but are bounded by the rolling
  window + retention policy). Future cleanup work fits within the
  existing storage-side retention machinery.
- **N=3-in-1h threshold is a heuristic.** A noisy IdP combined with a
  too-tight TTL could trigger false escalations to `ReauthRequired` in
  the long tail. The threshold and window are config knobs. Defaults
  err on the side of fewer escalations; deployment guidance documents
  tuning.
- **`tokio::spawn` of a heartbeat task per active claim.** Bounded by
  the number of credentials refreshing concurrently, which is small —
  the same data structure already bounds in-flight refreshes via the
  L1 LRU. Future audit if production traffic shifts.

### Neutral

- **`RefreshCoordinator` is still a concrete primitive, not a trait.**
  ADR-0030 §3's rationale (no broken-impl surface area) holds — the
  primitive is now composed of two halves, but neither half is
  user-pluggable beyond the storage seam. The `RefreshClaimRepo` trait
  is the storage seam, not a coordination-strategy seam. Plugging in a
  third coordination tier (e.g., cross-region) would open a new ADR.
- **`Credential::refresh()` trait method unchanged.** This ADR wraps
  the existing seam; it does not change what happens *inside* refresh.
  ADR-0028 invariant 2 (§13.2 seam integrity) preserved.

## Alternatives considered

The full rejection rationale lives in spec §8.2; the load-bearing
summaries:

### A. External coordinator (etcd / ZooKeeper / Consul)

**Rejected.** Nebula's local-first stance (canon §12.3) treats external
coordination services as deployment burden incompatible with the
desktop / self-hosted default. Every external coordinator is one more
process to install, monitor, secure, and reason about under failure. The
in-storage CAS approach has zero new dependencies for any deployment
that already runs Postgres or SQLite (which is every deployment).

### B. Postgres advisory locks (`pg_advisory_lock`)

**Rejected.** Postgres-only. SQLite has no equivalent, so adopting
advisory locks would mean *different* coordination semantics in desktop
vs production modes. Spec parity (§7.3) would degrade and
desktop-to-production drift would compound. The CAS+TTL pattern works
identically across both dialects.

### C. Accept the race; document as known limitation

**Rejected.** Canon §4.5 (operational honesty) is the binding rule.
n8n #13088 is a confirmed two-year-old production issue in a comparable
project; shipping the same shape as a "known limitation" while marketing
multi-replica engine support is a §11.6 false capability. The cost of
the fix (one storage trip per refresh, two new tables) is
demonstrably smaller than the cost of the bug.

### D. Single-writer replica election (one replica refreshes for all)

**Rejected.** Hot-spot on the leader; leader failure stalls all
refreshes until re-election; introduces another distributed-coordination
axis (election + leader liveness detection) for marginal gain. The
two-tier per-credential approach scales horizontally without coupling
all credential refreshes to one replica's health.

## Schema overview note (forward reference)

This section deliberately stays high-level — the migration body is the
implementation seam, not the ADR seam. The migrations land in
`crates/storage/migrations/{sqlite,postgres}/` with the П2 Stage 1 PR.
The schema-parity CI check (existing) gates that both dialects evolve
together. If a future schema revision is needed (e.g., adding a column
for a new failure mode), it lands as its own migration with an ADR
appendix or supersession, per
[ADR README](./README.md) — *"Do not substantively edit an accepted
ADR. Open a new one with `supersedes: [NNNN]`."*

## Cross-references

- **Sub-spec**:
  `docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md`
  — full design (sequence diagrams, schema DDL, test plan, observability
  surface, parameter tuning guidance, open questions).
- **Plan**:
  `docs/superpowers/plans/2026-04-26-credential-p2-refresh-coordination-l2.md`
  — staged implementation (this ADR is Stage 0; Stages 1–5 land the
  storage trait, engine refactor, sentinel logic, observability, doc
  sync).
- **ADR-0030**: `0030-engine-owns-credential-orchestration.md` §3 — the
  amendment site. ADR-0030 names this ADR's class as the trigger for
  superseding §3's "concrete, not trait" decision; that decision now
  reads as "concrete primitive composed of L1 (in-process) + L2 (storage
  seam — `RefreshClaimRepo` trait)."
- **ADR-0028**: `0028-cross-crate-credential-invariants.md` invariants 2
  (§13.2 seam) and 5 (Engine integration MATURITY).
- **ADR-0008**: `0008-execution-control-queue-consumer.md` §1 — the
  CAS+claim+ack pattern this ADR mirrors.
- **ADR-0017**: `0017-control-queue-reclaim-policy.md` — the
  TTL+heartbeat+reclaim pattern this ADR mirrors. Same retry-budget
  shape; same multi-runner safety via CAS-on-row.
- **n8n field report**: `docs/research/n8n-credential-pain-points.md`
  §1 — the production race in a comparable project that motivates this
  ADR's existence.
