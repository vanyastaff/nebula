---
name: credential concurrency correctness — fix 4 race conditions surfaced by ChatGPT round-2 review
status: draft (writing-skills output 2026-04-27)
date: 2026-04-27
authors: [vanyastaff, Claude]
phase: prod-blocking parallel-track (does not block П3 kickoff but should land before П3 lifecycle work)
scope: cross-cutting — nebula-engine (executor, resolver), nebula-storage (credential cache + pending), nebula-credential (PendingStore contract)
related:
  - docs/tracking/credential-audit-2026-04-27.md §XII Errata (review round 1)
  - ChatGPT 5.5 review round 2 (2026-04-27) — findings #1, #2, #3, #6
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §3.1 (PendingStore single-use contract)
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md §15.10 (PendingStore::consume atomicity)
  - docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md (refresh coordinator L1/L2)
  - docs/adr/0029-storage-owns-credential-persistence.md (CAS contract)
  - docs/adr/0030-engine-owns-credential-orchestration.md (resolver lifecycle)
defers-to: none
---

# Credential Concurrency Correctness — fix 4 race conditions

## §0 Meta

**Scope.** Fix 4 distinct concurrency / race bugs surfaced by ChatGPT 5.5 round-2 review (2026-04-27). All 4 are bona fide correctness issues, not hardening: each is reproducible by a concurrent test fixture and produces incorrect behavior in absence of an attacker.

**Bugs in scope:**
- **CC-01 — execute_continue replay.** Two concurrent OAuth callbacks both enter `Interactive::continue_resolve` before single-use enforcement; both can exchange the same auth code and produce duplicate side-effects.
- **CC-02 — reauth_required ignored on resolve.** `load_and_verify` does not check `reauth_required = true`; once a provider rejects refresh, the engine keeps handing out the rejected credential as valid.
- **CC-03 — CAS retry stale overwrite.** After `VersionConflict`, `perform_refresh` updates `expected_version` to the new value but writes a `StoredCredential` body cloned from the stale row → optimistic concurrency degrades to stale overwrite.
- **CC-06 — Cache stale overwrite race.** A cold `get` loading slowly can insert an old row into the cache after a concurrent `put` has already cached a newer row, hiding refresh publication behind stale data until TTL expiry.

**Non-goals.**
- OAuth2 lifecycle error mapping (CC-04 — `invalid_grant` → `ReauthRequired`) — separate spec `2026-04-27-credential-oauth2-lifecycle-correctness-design.md`.
- Capability reporting honesty (CC-07) and owner fail-open (CC-09) — same OAuth2 lifecycle spec.
- `StoredCredential.data: Vec<u8>` ambiguity (CC-08) and built-in crate split (CC-10) — П3 scope or own dedicated specs.
- Performance optimization beyond what is required to fix correctness.

**Phase.** **Prod-blocking parallel-track.** Each bug produces incorrect behavior under realistic concurrent load (multi-replica or concurrent OAuth callbacks). Should land before any new credential lifecycle work in П3 to avoid building on a racy substrate. Does not block П3 *kickoff* (specs/scaffolding) but does block П3 *integration test* of credential lifecycle.

**Reading order.** §0 (this) → §1 (per-bug catalog) → §2-§5 (per-bug stage design) → §6 (doc sync) → §7 (tests) → §8 (acceptance) → §9 (deferred) → §10 (migration).

## §1 Bug catalog

### §1.1 CC-01 — execute_continue replay

| Field | Value |
|---|---|
| Severity | HIGH |
| Location | `crates/engine/src/credential/executor.rs::execute_continue` |
| Pattern | `get_bound(token, ...)` → `continue_resolve(state, input)` → `pending.consume(token)` |
| Race window | Between `get_bound` and `consume`; covers `continue_resolve` (which may include external IdP roundtrip) |
| Concrete scenario | Two OAuth callbacks arrive for the same pending token (replay or duplicate redirect). Both pass `get_bound` (returns same `PendingState`). Both call `continue_resolve`, which in OAuth2 case exchanges the auth code with the IdP. The IdP rejects the second exchange; the first succeeds. Then one consume succeeds; the other fails. Duplicate side-effect already happened. |
| Why structural enforcement | Tech Spec §3.1 documents PendingStore as «single-use», but the engine enforces single-use AFTER the side-effect, not before. |

### §1.2 CC-02 — reauth_required ignored on resolve

| Field | Value |
|---|---|
| Severity | HIGH |
| Location | `crates/engine/src/credential/resolver.rs::load_and_verify` and `resolve_with_refresh` |
| Pattern | `load_row` returns `StoredCredential` containing a `reauth_required` flag (or equivalent metadata); resolver path does not check it |
| Race window | After provider rejects refresh and another path sets `reauth_required = true`, ANY subsequent resolve before the operator re-authenticates returns the rejected credential as if valid |
| Concrete scenario | Replica A receives `invalid_grant` from IdP, sets `reauth_required = true` on the row. Replica B, after `RefreshOutcome::CoalescedByOtherReplica`, calls `resolve` again, which loads the row and returns the stored (rejected) token as `CredentialHandle`. Caller uses it; downstream IdP request fails again. |
| Why structural enforcement | Tech Spec implies `reauth_required` blocks projected-auth access; code does not enforce. |

### §1.3 CC-03 — CAS retry stale overwrite

| Field | Value |
|---|---|
| Severity | HIGH |
| Location | `crates/engine/src/credential/resolver.rs::perform_refresh` |
| Pattern | Read row at version `v1`; do refresh work (HTTP); attempt CAS write with `expected_version = v1`; on `VersionConflict`, update `expected_version = v2`; retry write with same body |
| Race window | Between the original read at `v1` and the retry write — concurrent worker has written `v2` with different content (e.g., metadata update, scope change, reauth_required flag) |
| Concrete scenario | Worker X reads v1 of credential C, starts OAuth refresh. Worker Y writes v2 of C (operator updates scopes). X finishes refresh, attempts CAS at v1 — conflict. X updates `expected_version` to v2 and writes its body (containing v1 metadata, new tokens) — store accepts. Y's scope update is lost; refresh tokens published. |
| Why structural enforcement | ADR-0029 documents CAS as «no clobber concurrent updates». Current retry violates the contract. |

### §1.4 CC-06 — Cache stale overwrite race

| Field | Value |
|---|---|
| Severity | HIGH |
| Location | `crates/storage/src/credential/layer/cache.rs::CacheLayer::get` and `CacheLayer::put` |
| Pattern | `get` on cache miss → `inner_store.get(id)` → on return, `cache.insert(id, row)`; concurrent `put` updates `inner_store` and calls `cache.insert(id, new_row)` |
| Race window | Between `inner_store.get` returning a stale read and the cache insert; concurrent `put` may have already published a newer row to cache |
| Concrete scenario | Task A: `cache.get(id)` miss → `inner.get(id)` returns v1 (slow because backend was busy). Task B: `inner.put(id, v2)` succeeds → `cache.insert(id, v2)`. Task A resumes → `cache.insert(id, v1)`. Cache now has v1; v2 publication hidden until TTL expiry. |
| Why structural enforcement | Refresh publication ordering is the foundation of «refresh credential is visible to all readers». Stale cache violates the cross-replica contract. |

## §2 Stage 0 — Investigation + reproducible tests

**Goal.** Reproduce each bug with a deterministic test fixture before committing fixes. No code changes; PR is investigation-only.

### Tasks

- [ ] **CC-01 reproducer:** `crates/engine/tests/cc01_execute_continue_replay.rs`
  - Fixture credential type with `Interactive::continue_resolve` that increments `AtomicUsize` behind a `tokio::sync::Barrier`.
  - Two `tokio::spawn` tasks call `execute_continue` with the same token; barrier syncs them past `get_bound` simultaneously.
  - Assert: counter == 2 (the bug). Stage 1 fix flips this to == 1.

- [ ] **CC-02 reproducer:** `crates/engine/tests/cc02_reauth_required_ignored.rs`
  - Seed a stored credential row with `reauth_required = true`.
  - Call `resolve` directly and via `RefreshOutcome::CoalescedByOtherReplica` path.
  - Assert: returns `Ok(CredentialHandle)` (the bug). Stage 2 fix flips this to `Err(ResolveError::ReauthRequired)`.

- [ ] **CC-03 reproducer:** `crates/engine/tests/cc03_cas_retry_stale_overwrite.rs`
  - Scripted store impl that returns `VersionConflict` on first write, then accepts second write.
  - Between conflict and retry, another «virtual» worker has updated the row (state == different metadata).
  - Assert: post-retry row contains stale metadata (the bug). Stage 3 fix preserves the concurrent metadata.

- [ ] **CC-06 reproducer:** `crates/storage/tests/cc06_cache_stale_overwrite_race.rs`
  - Controlled `inner_store` with barriers forcing the get-vs-put interleave.
  - Assert: final `cache.get(id)` returns v1 (the bug). Stage 4 fix returns v2.

**Stage 0 landing gate:** all 4 reproducers compile and **fail** (assertions hit the buggy behavior). They become the regression tests for Stages 1-4.

## §3 Stage 1 — CC-01 fix: claim-before-side-effect on PendingStore

**Goal.** Make `execute_continue` atomic from PendingStore's perspective. Two concurrent callers can never both enter `continue_resolve` for the same token.

**Design — extend `PendingStore` contract:**

Add new method:
```rust
pub trait PendingStore: Send + Sync {
    /// Atomically transition pending entry to `Claimed` state.
    /// Returns the bound state if claim succeeded; `Err(AlreadyClaimed)` if
    /// another caller has already claimed the same token.
    /// Claim has TTL of CLAIM_HOLD_SECS (60 seconds default); if `complete_consume`
    /// or `release_claim` is not called within TTL, the claim expires and
    /// another caller can claim again.
    async fn claim_for_continue(
        &self,
        token: &PendingToken,
        binding: &PendingBinding,
    ) -> Result<PendingState, ClaimError>;

    /// Finalize a claimed entry — atomically delete from storage.
    /// Called after `continue_resolve` succeeds.
    async fn complete_consume(&self, token: &PendingToken) -> Result<(), ClaimError>;

    /// Release a claim without deleting (e.g., continue_resolve returned a
    /// non-terminal error and another caller may retry once TTL expires).
    async fn release_claim(&self, token: &PendingToken) -> Result<(), ClaimError>;
}
```

`execute_continue` flow becomes:
```rust
let pending_state = pending.claim_for_continue(&token, &binding).await?;
match credential.continue_resolve(pending_state, input).await {
    Ok(new_state) => {
        pending.complete_consume(&token).await?;
        // proceed with state persistence
    }
    Err(e) if e.is_terminal() => {
        pending.complete_consume(&token).await?;
        return Err(e.into());
    }
    Err(e) => {
        pending.release_claim(&token).await?;
        return Err(e.into());
    }
}
```

**Implementation impact:**
- Storage layer: adds `claimed_until: Option<Timestamp>` column to pending table; CAS-based claim transition.
- In-memory PendingStore: claim is a `HashMap<PendingToken, ClaimState>` with mutex per token.
- SQLite + Postgres: `UPDATE pending_credentials SET claimed_until = NOW() + INTERVAL '60 seconds' WHERE token = $1 AND (claimed_until IS NULL OR claimed_until < NOW())` — atomic claim via row-level lock.

**Landing gate (Stage 1):**
- CC-01 reproducer flips green (counter == 1).
- New unit tests for `claim_for_continue` cover: successful claim, claim collision, TTL expiry, release-claim semantics.
- No regression in existing pending-store integration tests.
- Commit (squash): `feat(credential)!: CC-01 — pending claim before continue (Stage 1)`

## §4 Stage 2 — CC-02 fix: reauth_required propagation

**Goal.** When `reauth_required = true` on a stored credential, all resolve paths return `ResolveError::ReauthRequired` instead of the rejected credential.

**Design.**

In `resolver.rs::load_and_verify` (and `resolve_with_refresh`):
```rust
let stored = storage.get(id).await?;
if stored.reauth_required {
    return Err(ResolveError::ReauthRequired {
        reason: stored.reauth_reason.clone()
            .unwrap_or(ReauthReason::Unknown),
        credential_id: id.clone(),
    });
}
// existing flow
```

In `resolve_with_refresh` after `RefreshOutcome::CoalescedByOtherReplica`:
```rust
RefreshOutcome::CoalescedByOtherReplica => {
    // re-load to pick up the result (or the reauth flag);
    // do NOT short-circuit return CredentialHandle from cached state
    let stored = storage.get(id).await?;
    if stored.reauth_required {
        return Err(ResolveError::ReauthRequired { ... });
    }
    // proceed with handle from fresh row
}
```

Persist reason metadata: add `reauth_reason: Option<ReauthReason>` to `StoredCredential`. Reason values: `InvalidGrant`, `InvalidRefreshToken`, `Revoked`, `SentinelRepeatedThreshold`, `Unknown`.

**Landing gate (Stage 2):**
- CC-02 reproducer flips green (returns `ReauthRequired`).
- Test for `RefreshOutcome::CoalescedByOtherReplica` after concurrent `invalid_grant` flag-set: returns `ReauthRequired`, not stale handle.
- New unit test: direct resolve with `reauth_required = true` → `ReauthRequired`.
- Commit (squash): `feat(credential)!: CC-02 — reauth_required propagation (Stage 2)`

## §5 Stage 3 — CC-03 fix: CAS retry merge-on-conflict

**Goal.** On `VersionConflict`, re-read the row and re-apply only the fields owned by the refresh path; never blind-overwrite a concurrent update.

**Design.**

`perform_refresh` retry loop becomes:
```rust
loop {
    let stored = storage.get(id).await?;
    let expected_version = stored.version;
    if stored.reauth_required {
        return Err(ResolveError::ReauthRequired { ... }); // CC-02 cross-cutting
    }
    let refresh_outcome = run_refresh(stored.clone()).await?;
    let mut updated = stored.clone();
    // Refresh path owns: access_token, refresh_token (rotated), expires_at, scopes (returned by IdP)
    apply_refresh_outcome(&mut updated, &refresh_outcome);
    match storage.put(id, updated, expected_version).await {
        Ok(_) => break,
        Err(StorageError::VersionConflict) => continue, // re-read + re-apply
        Err(e) => return Err(e.into()),
    }
}
```

Critically: the retry **re-reads the row** every iteration. The «refresh-path-owned fields» are the ones the IdP just returned — those are merged onto the fresh row. Any field the resolver does not own (operator-set scopes, metadata, reauth_required flag, etc.) is preserved from the concurrent write.

If the retry exceeds `MAX_REFRESH_RETRIES` (e.g., 3), abort with `ResolveError::ConcurrencyExceeded`; do not silently overwrite.

**Landing gate (Stage 3):**
- CC-03 reproducer flips green (concurrent update preserved).
- New unit test: `MAX_REFRESH_RETRIES` exhaustion returns `ConcurrencyExceeded`.
- Commit (squash): `fix(engine): CC-03 — CAS retry merge-on-conflict (Stage 3)`

## §6 Stage 4 — CC-06 fix: cache version-aware population + per-key single-flight

**Goal.** Cache cannot regress to a stale row after a fresh row has been published.

**Design.**

`CacheLayer::get` populates cache only when the row's version is strictly greater than what is already cached:
```rust
pub async fn get(&self, id: &CredentialId) -> Result<Option<StoredCredential>, _> {
    if let Some(cached) = self.cache.peek(id) {
        return Ok(Some(cached));
    }
    // single-flight: per-key mutex guards the inner read
    let _guard = self.singleflight.acquire(id).await;
    if let Some(cached) = self.cache.peek(id) {
        return Ok(Some(cached));
    }
    let row = self.inner.get(id).await?;
    if let Some(row) = row {
        self.cache.insert_if_newer(id, row.clone()); // version-aware
        Ok(Some(row))
    } else {
        Ok(None)
    }
}
```

`insert_if_newer` is atomic: if existing entry has version >= incoming, drop the incoming. If incoming is newer, replace.

`CacheLayer::put` similarly uses `insert_if_newer` (defensive) and acquires the per-key single-flight to serialize the put with concurrent gets.

**Landing gate (Stage 4):**
- CC-06 reproducer flips green (cache returns v2 after race).
- New stress test: 100 concurrent `get`/`put` pairs against scripted backend; final cached state matches the latest version written.
- Commit (squash): `fix(storage): CC-06 — cache version-aware population (Stage 4)`

## §7 Stage 5 — Doc sync

- `docs/MATURITY.md`: append «Concurrency correctness 2026-04-27 (CC-cluster) (PR `<sha>`)» under `nebula-engine` and `nebula-storage` Audited columns.
- `docs/OBSERVABILITY.md`: add metric `credential.refresh.cas_retry_total` (counter; labels: outcome ∈ {success, conflict_retry, concurrency_exceeded}); add span attr `credential.cache.stale_drop_total` for `insert_if_newer` rejection.
- `docs/UPGRADE_COMPAT.md`: add row for breaking changes (PendingStore trait extended, `StoredCredential.reauth_reason` added).
- `docs/tracking/credential-concerns-register.md`: add 4 new rows (CC-01, CC-02, CC-03, CC-06) with `decided` status and PR SHA.
- `docs/superpowers/specs/2026-04-24-credential-tech-spec.md`: amendment note in §3.1 (PendingStore claim-before-side-effect) + §15.10 (claim semantics extension).
- `CHANGELOG.md`: entry under unreleased «Security/Correctness» — 4 race-condition fixes.

## §8 Test strategy

**Reproducer regressions (mandatory landing gates per Stage 1-4):**
- `cc01_execute_continue_replay.rs` — barrier-driven, asserts counter == 1
- `cc02_reauth_required_ignored.rs` — direct + coalesced paths return `ReauthRequired`
- `cc03_cas_retry_stale_overwrite.rs` — retry preserves concurrent update
- `cc06_cache_stale_overwrite_race.rs` — cache returns latest version

**New unit tests:**
- PendingStore: `claim_for_continue_collision`, `claim_ttl_expiry`, `release_claim_allows_retry`
- Resolver: `cas_retry_max_exceeded`
- CacheLayer: `insert_if_newer_drops_stale`, `singleflight_serializes`

**Loom or stress tests (optional but recommended):**
- `cc06_cache_loom.rs` — loom model checker for get/put interleaving (if existing infra supports loom; else skip).

## §9 Acceptance criteria per stage

| Stage | Acceptance |
|---|---|
| 0 | 4 reproducers compile and fail (committed before fixes) |
| 1 | CC-01 reproducer green; PendingStore unit tests pass |
| 2 | CC-02 reproducer green; both direct + coalesced paths covered |
| 3 | CC-03 reproducer green; concurrency-exceeded path tested |
| 4 | CC-06 reproducer green; stress test stable |
| 5 | All 6 docs synced; register rows added |

**Spec-level DoD:**
- All 4 reproducers green (post-fix).
- All new unit tests green.
- `cargo nextest run --workspace` no regression.
- `cargo clippy --workspace -- -D warnings` green.
- Tech Spec §3.1 + §15.10 amendments applied.

## §10 Deferred / parallel tracks

| Item | Forward-pointer | Reason |
|---|---|---|
| CC-04 (`invalid_grant` → `ReauthRequired` mapping) | `2026-04-27-credential-oauth2-lifecycle-correctness-design.md` | Different axis (OAuth2 lifecycle vs concurrency) |
| CC-07 (false capability reporting) | `2026-04-27-credential-oauth2-lifecycle-correctness-design.md` | Same as above |
| CC-09 (`owner_id()` fail-open) | `2026-04-27-credential-oauth2-lifecycle-correctness-design.md` | Identity boundary, OAuth2-adjacent |
| CC-08 (`StoredCredential.data: Vec<u8>` ambiguity) | П3 architectural scope (separate spec TBD) | Type design refactor, larger impact |
| CC-10 (built-in crate split) | П3 architectural scope (separate spec TBD) | Crate organization |
| Cross-replica chaos test | П2 plan Stage 4 (already in flight) | Existing test surface |

## §11 Migration / rollout

**Breaking changes (active dev mode):**
- `PendingStore` trait extended with 3 new methods (`claim_for_continue`, `complete_consume`, `release_claim`). Existing impls must be updated.
- `StoredCredential` adds `reauth_required: bool` and `reauth_reason: Option<ReauthReason>` fields (already present per ChatGPT review; verify in Stage 0).
- `ResolveError` adds `ReauthRequired` and `ConcurrencyExceeded` variants. Existing matches break — explicit handling required.

**Rollout:** Stages 1-4 produce 4 separate squash-merge commits. Stage 5 is a single commit. Each stage independently revertable.

**Storage migrations:** Stage 1 adds `claimed_until` column to pending table — 2 new sqlx migrations (sqlite + postgres). Idempotent; safe to apply on running deployment.

---

**Spec complete.** Implementation plan to follow per writing-plans skill.
