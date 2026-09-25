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

#[cfg(test)]
use super::RefreshOutcomeDecision;
use super::{
    ClaimAttempt, ClaimToken, CredentialIncidentRef, CredentialOperationDecision,
    CredentialOperationIntent, CredentialOperationKind, HeartbeatError, RefreshAdjudication,
    RefreshClaim, RefreshClaimAdjudicationError as RepoAdjudicationError, RefreshClaimAdjudicator,
    RefreshClaimRepo, ReplicaId, RepoError, SentinelState, adjudicate_against_recorded_resolution,
    adjudication_evidence_digest, validate_adjudication_evidence,
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
    operation: CredentialOperationKind,
}

/// What an operator decided about an incident's unknown provider outcome.
///
/// Its presence *is* the resolution: the SQL backends need an
/// `adjudicated_at` column to say "resolved", while `Option` says it here
/// without a second field that could disagree.
#[derive(Clone, Debug)]
struct SentinelAdjudication {
    decision: CredentialOperationDecision,
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
    operation: CredentialOperationKind,
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
        intent: CredentialOperationIntent,
    ) -> Result<ClaimAttempt, RepoError> {
        if matches!(intent, CredentialOperationIntent::Revoke { .. }) {
            return Err(RepoError::AggregateUnavailable);
        }
        let operation = intent.kind();
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
                    operation: existing.operation,
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
            operation,
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
        incident: CredentialIncidentRef,
        decision: CredentialOperationDecision,
        evidence: &str,
    ) -> Result<RefreshAdjudication, RepoAdjudicationError> {
        validate_adjudication_evidence(evidence)?;
        let digest = adjudication_evidence_digest(evidence);
        let incident = incident.as_uuid();

        // Same lock order as `reclaim_stuck` and `release` (claims, then
        // incidents): clearing the poison and recording its resolution share
        // one critical section, so no observer sees a cleared claim without
        // its outcome, and no two adjudications can both find the row.
        let mut claims = self.inner.lock();
        let mut events = self.sentinel_events.lock();
        let now = self.clock.now();

        // The named incident's own resolution answers first, so a retry after
        // a lost acknowledgement cannot fall through to a newer poisoned claim
        // its decision never described.
        let incident_index = events
            .iter()
            .position(|event| event.selector == *selector && event.claim_id == incident);
        if let Some(index) = incident_index
            && let Some(recorded) = events[index].adjudication.as_ref()
        {
            return adjudicate_against_recorded_resolution(
                &recorded.evidence_digest,
                Some(recorded.decision.as_str()),
                events[index].operation,
                &digest,
                decision,
            );
        }

        // The poison is the claim row and not its accounting: the sweep may not
        // have run yet, and `try_claim` refuses egress on the row. Same
        // predicate, same clock as `try_claim`, so a caller that just observed
        // `OutcomeUnknown` can always adjudicate what it saw.
        let poisoned = claims
            .get(selector)
            .filter(|row| row.expires_at < now && row.sentinel == SentinelState::RefreshInFlight)
            .map(|row| {
                (
                    row.claim_id,
                    row.holder.clone(),
                    row.generation,
                    row.operation,
                )
            });
        let Some((claim_id, crashed_holder, generation, operation)) = poisoned else {
            return Err(RepoAdjudicationError::NotPoisoned);
        };
        if claim_id != incident {
            return Err(RepoAdjudicationError::StaleIncident);
        }
        if operation != decision.kind() {
            return Err(RepoAdjudicationError::OperationMismatch {
                recorded_operation: operation,
            });
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
                events.push(SentinelEventRow {
                    selector: selector.clone(),
                    claim_id,
                    detected_at: now,
                    crashed_holder,
                    generation,
                    operation,
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
