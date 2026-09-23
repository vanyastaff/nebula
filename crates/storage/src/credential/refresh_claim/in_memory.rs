//! In-memory `RefreshClaimRepo` impl for tests + desktop-mode fallback.
//!
//! Single-process scope — no cross-replica coordination. CAS uses a
//! `parking_lot::Mutex` over `HashMap<CredentialSelector, ClaimRow>`.

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use nebula_core::accessor::{Clock, SystemClock};
use nebula_storage_port::CredentialSelector;
use parking_lot::Mutex;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, HeartbeatError, RefreshAdjudication, RefreshClaim,
    RefreshClaimAdjudicationError as RepoAdjudicationError, RefreshClaimAdjudicator,
    RefreshClaimRepo, RefreshOutcomeDecision, ReplicaId, RepoError, SentinelState,
    adjudicate_against_recorded_resolution, adjudication_evidence_digest,
    validate_adjudication_evidence,
};

#[derive(Clone, Debug)]
struct ClaimRow {
    claim_id: Uuid,
    generation: u64,
    holder: ReplicaId,
    #[expect(dead_code, reason = "kept for future event/metric emission")]
    acquired_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    sentinel: SentinelState,
}

/// What an operator decided about an incident's unknown provider outcome.
///
/// Its presence *is* the resolution: the SQL backends need an
/// `adjudicated_at` column to say "resolved", while `Option` says it here
/// without a second field that could disagree.
#[derive(Clone, Debug)]
struct SentinelAdjudication {
    decision: RefreshOutcomeDecision,
    #[expect(dead_code, reason = "retained as incident observability evidence")]
    evidence: String,
    evidence_digest: [u8; 32],
}

/// One sentinel event record kept in the in-memory ring.
#[derive(Clone, Debug)]
struct SentinelEventRow {
    selector: CredentialSelector,
    claim_id: Uuid,
    #[expect(dead_code, reason = "retained as incident observability evidence")]
    detected_at: DateTime<Utc>,
    #[expect(dead_code, reason = "retained as incident observability evidence")]
    crashed_holder: ReplicaId,
    #[expect(dead_code, reason = "retained as incident observability evidence")]
    generation: u64,
    /// `None` until an adjudication records the provider outcome. This is the
    /// in-memory analogue of `credential_sentinel_events.adjudicated_at`: an
    /// incident without it is unresolved poison.
    adjudication: Option<SentinelAdjudication>,
}

/// In-memory `RefreshClaimRepo`. Cheap to clone (Arc-backed inner).
#[derive(Clone)]
pub struct InMemoryRefreshClaimRepo {
    inner: Arc<Mutex<HashMap<CredentialSelector, ClaimRow>>>,
    sentinel_events: Arc<Mutex<Vec<SentinelEventRow>>>,
    /// This adapter's clock, and the **only** clock the reconciliation
    /// mechanism accepts from a caller.
    ///
    /// Expiry is a claim about liveness, so the SQL backends decide it in the
    /// database (`unixepoch('now')`, `CURRENT_TIMESTAMP`) and no Rust clock can
    /// reach their predicates. This adapter serves exactly one process, so it
    /// has no shared authority to protect and is the correct place to hand
    /// time-travelling tests a lever.
    clock: Arc<dyn Clock>,
}

impl Default for InMemoryRefreshClaimRepo {
    fn default() -> Self {
        Self {
            inner: Arc::default(),
            sentinel_events: Arc::default(),
            clock: Arc::new(SystemClock),
        }
    }
}

impl InMemoryRefreshClaimRepo {
    /// Create a fresh, empty repo.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a fresh, empty repo driven by `clock`.
    ///
    /// For tests that need a claim to expire without sleeping.
    #[must_use]
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Arc::default(),
            sentinel_events: Arc::default(),
            clock,
        }
    }
}

impl std::fmt::Debug for InMemoryRefreshClaimRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryRefreshClaimRepo")
            .field("entries", &self.inner.lock().len())
            .field("sentinel_events", &self.sentinel_events.lock().len())
            .finish()
    }
}

#[async_trait::async_trait]
impl RefreshClaimRepo for InMemoryRefreshClaimRepo {
    async fn try_claim(
        &self,
        selector: &CredentialSelector,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        let mut guard = self.inner.lock();
        let now = self.clock.now();

        if let Some(existing) = guard.get(selector) {
            if existing.expires_at >= now {
                return Ok(ClaimAttempt::Contended {
                    existing_expires_at: existing.expires_at,
                });
            }
            // Crossing the provider boundary changes expiry from ordinary
            // lease loss into durable poison. No caller may retry provider
            // egress until an explicit reconciliation command clears it.
            if existing.sentinel == SentinelState::RefreshInFlight {
                return Ok(ClaimAttempt::OutcomeUnknown {
                    expired_at: existing.expires_at,
                });
            }
        }
        // No row OR an expired Normal row — claim wins. Generation bumps
        // if we're overwriting.

        let claim_id = Uuid::new_v4();
        let generation = guard.get(selector).map_or(0, |row| row.generation + 1);
        let acquired_at = now;
        let expires_at =
            now + chrono::Duration::from_std(ttl).map_err(|_| RepoError::InvalidState)?;

        let row = ClaimRow {
            claim_id,
            generation,
            holder: holder.clone(),
            acquired_at,
            expires_at,
            // The overwrite predicate above admits only Normal rows, so
            // this reset cannot erase unaccounted in-flight evidence.
            sentinel: SentinelState::Normal,
        };
        guard.insert(selector.clone(), row);

        Ok(ClaimAttempt::Acquired(RefreshClaim {
            selector: selector.clone(),
            token: ClaimToken {
                selector: selector.clone(),
                claim_id,
                generation,
            },
            acquired_at,
            expires_at,
        }))
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        let extension = chrono::Duration::from_std(ttl)
            .map_err(|_| HeartbeatError::Repo(RepoError::InvalidState))?;
        let mut guard = self.inner.lock();
        let now = self.clock.now();

        let row = guard
            .get_mut(&token.selector)
            .filter(|row| row.claim_id == token.claim_id && row.generation == token.generation);
        match row {
            Some(r) if r.expires_at > now => {
                // Extend by `ttl` past now. Caller (RefreshCoordinator) must
                // pass the configured `claim_ttl` so the §3.5 invariants hold.
                r.expires_at = now + extension;
                Ok(())
            },
            _ => Err(HeartbeatError::ClaimLost),
        }
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError> {
        // Same lock order as `reclaim_stuck` (claims, then incidents): the
        // incident check and the delete are one critical section, so an
        // adjudication cannot land between them.
        let mut guard = self.inner.lock();
        let events = self.sentinel_events.lock();

        // An incident with no recorded provider outcome is unresolved poison:
        // it outlives its claim row until `adjudicate` decides it, so the row
        // is retained and the caller learns why. An absent claim, a superseded
        // generation, or an already-reconciled incident all keep release
        // idempotent.
        if events
            .iter()
            .any(|event| event.claim_id == token.claim_id && event.adjudication.is_none())
        {
            return Err(RepoError::ReleaseRefused);
        }

        if guard
            .get(&token.selector)
            .is_some_and(|row| row.claim_id == token.claim_id && row.generation == token.generation)
        {
            guard.remove(&token.selector);
        }
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        let mut guard = self.inner.lock();
        // Read time while holding the same mutex that protects the state
        // transition. Otherwise a task paused between reading the clock and
        // locking the row could mark a claim that expired meanwhile.
        let now = self.clock.now();
        let row = guard
            .get_mut(&token.selector)
            .filter(|row| row.claim_id == token.claim_id && row.generation == token.generation);
        // Mirrors heartbeat's claim-validity check: an absent or expired row
        // no longer authorizes provider egress. Silently succeeding would
        // let a holder whose TTL elapsed proceed to the IdP POST while the
        // row is eligible for reclaim.
        match row {
            Some(r) if r.expires_at > now => {
                r.sentinel = SentinelState::RefreshInFlight;
                Ok(())
            },
            _ => Err(RepoError::InvalidState),
        }
    }
}

#[async_trait::async_trait]
impl RefreshClaimAdjudicator for InMemoryRefreshClaimRepo {
    async fn adjudicate(
        &self,
        selector: &CredentialSelector,
        decision: RefreshOutcomeDecision,
        evidence: &str,
    ) -> Result<RefreshAdjudication, RepoAdjudicationError> {
        validate_adjudication_evidence(evidence)?;
        let digest = adjudication_evidence_digest(evidence);

        // Same lock order as `reclaim_stuck` and `release` (claims, then
        // incidents): clearing the poison and recording its resolution share
        // one critical section, so no observer sees a cleared claim without
        // its outcome, and no two adjudications can both find the row.
        let mut claims = self.inner.lock();
        let mut events = self.sentinel_events.lock();
        let now = self.clock.now();

        // The poison is the claim row and not its accounting: the sweep may not
        // have run yet, and `try_claim` refuses egress on the row. Same
        // predicate, same clock as `try_claim`, so a caller that just observed
        // `OutcomeUnknown` can always adjudicate what it saw.
        let poisoned = claims
            .get(selector)
            .filter(|row| row.expires_at < now && row.sentinel == SentinelState::RefreshInFlight)
            .map(|row| (row.claim_id, row.holder.clone(), row.generation));

        let Some((claim_id, crashed_holder, generation)) = poisoned else {
            // Nothing is poisoned. The credential's resolved set is the only
            // identity available — there is no claim row, and the request
            // carries none — so the rule is an exact match over what the
            // credential has already decided: this pair on record is the
            // idempotent recommit, a set without it contradicts every decision
            // on record, and an empty set has nothing to adjudicate. The ring
            // is append-only in chronological order, so the newest resolution
            // on record is the last resolved event: that is the pair a
            // refusal names, since a set has no single incident to point at.
            let resolved: Vec<&SentinelAdjudication> = events
                .iter()
                .filter(|event| event.selector == *selector)
                .filter_map(|event| event.adjudication.as_ref())
                .collect();
            let Some(newest) = resolved.last() else {
                return Err(RepoAdjudicationError::NotPoisoned);
            };
            let recommitted_pair = resolved.iter().any(|recorded| {
                recorded.evidence_digest == digest && recorded.decision == decision
            });
            if !recommitted_pair {
                return Err(RepoAdjudicationError::EvidenceConflict {
                    recorded_digest: newest.evidence_digest,
                    recorded_decision: newest.decision,
                });
            }
            return Ok(RefreshAdjudication::new(decision, false, digest));
        };

        // An incident that already carries a resolution decides this recommit,
        // and that decision can refuse. Resolve it before clearing the poison:
        // both SQL adapters roll a refused adjudication back inside their
        // transaction, and the replay denial has to survive one here too.
        //
        // A match is their committed delete, not a fall-through to the write
        // below: the recorded resolution already stands and equals this
        // request, so the claim row goes, the incident keeps its provenance,
        // and the replayed decision changes nothing. No port-reachable path
        // builds this state in any of the three adapters, so no test can fail
        // on it; the agreement is closed for the day it becomes reachable.
        let incident_index = events.iter().position(|event| event.claim_id == claim_id);
        if let Some(recorded) = incident_index.and_then(|index| events[index].adjudication.as_ref())
        {
            let recorded_adjudication = adjudicate_against_recorded_resolution(
                &recorded.evidence_digest,
                Some(recorded.decision.as_str()),
                &digest,
                decision,
            )?;
            claims.remove(selector);
            return Ok(recorded_adjudication);
        }

        claims.remove(selector);
        let resolution = SentinelAdjudication {
            decision,
            evidence: evidence.to_owned(),
            evidence_digest: digest,
        };

        // The incident is created here when the sweep has not run: the claim
        // row is the poison and the incident is its accounting, and they are
        // written at different times. An incident the sweep already recorded
        // keeps its provenance and detection time — the resolution written
        // here is the first one it carries.
        match incident_index {
            Some(index) => {
                events[index].adjudication = Some(resolution);
            },
            None => {
                // The sweep has not run yet, so the incident is created from
                // the claim row's own identity rather than awaited.
                events.push(SentinelEventRow {
                    selector: selector.clone(),
                    claim_id,
                    detected_at: now,
                    crashed_holder,
                    generation,
                    adjudication: Some(resolution),
                });
            },
        }
        Ok(RefreshAdjudication::new(decision, true, digest))
    }
}

#[cfg(test)]
#[path = "in_memory_tests.rs"]
mod tests;
