//! In-memory `RefreshClaimRepo` impl for tests + desktop-mode fallback.
//!
//! Single-process scope — no cross-replica coordination. CAS uses a
//! `parking_lot::Mutex` over `HashMap<CredentialId, ClaimRow>`.

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use nebula_core::accessor::{Clock, SystemClock};
use parking_lot::Mutex;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, RefreshAdjudication, RefreshClaim,
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
    credential_id: CredentialId,
    claim_id: Uuid,
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
    inner: Arc<Mutex<HashMap<CredentialId, ClaimRow>>>,
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
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        let mut guard = self.inner.lock();
        let now = self.clock.now();

        if let Some(existing) = guard.get(credential_id) {
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
        let generation = guard.get(credential_id).map_or(0, |row| row.generation + 1);
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
        guard.insert(*credential_id, row);

        Ok(ClaimAttempt::Acquired(RefreshClaim {
            credential_id: *credential_id,
            token: ClaimToken {
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
            .values_mut()
            .find(|r| r.claim_id == token.claim_id && r.generation == token.generation);
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

        guard.retain(|_, row| {
            !(row.claim_id == token.claim_id && row.generation == token.generation)
        });
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        let mut guard = self.inner.lock();
        // Read time while holding the same mutex that protects the state
        // transition. Otherwise a task paused between reading the clock and
        // locking the row could mark a claim that expired meanwhile.
        let now = self.clock.now();
        let row = guard
            .values_mut()
            .find(|r| r.claim_id == token.claim_id && r.generation == token.generation);
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

    async fn reclaim_stuck(&self) -> Result<Vec<ExpiredClaim>, RepoError> {
        let mut guard = self.inner.lock();
        let mut events = self.sentinel_events.lock();
        let now = self.clock.now();
        let mut out = Vec::new();

        let stuck: Vec<CredentialId> = guard
            .iter()
            .filter(|(credential_id, row)| {
                if row.expires_at >= now {
                    return false;
                }
                row.sentinel == SentinelState::Normal
                    || !events.iter().any(|event| {
                        event.credential_id == **credential_id && event.claim_id == row.claim_id
                    })
            })
            .map(|(k, _)| *k)
            .collect();

        for cid in stuck {
            let Some(row) = guard.get(&cid) else {
                continue;
            };
            match row.sentinel {
                SentinelState::Normal => {
                    let row = guard.remove(&cid).ok_or(RepoError::InvalidState)?;
                    out.push(ExpiredClaim::ReclaimedNormal {
                        credential_id: cid,
                        previous_holder: row.holder,
                        previous_generation: row.generation,
                    });
                },
                SentinelState::RefreshInFlight => {
                    events.push(SentinelEventRow {
                        credential_id: cid,
                        claim_id: row.claim_id,
                        detected_at: now,
                        crashed_holder: row.holder.clone(),
                        generation: row.generation,
                        // Accounting records the incident; only an adjudication
                        // may resolve it.
                        adjudication: None,
                    });
                    out.push(ExpiredClaim::OutcomeUnknownAccounted {
                        credential_id: cid,
                        previous_holder: row.holder.clone(),
                        previous_generation: row.generation,
                    });
                },
            }
        }

        Ok(out)
    }

    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &CredentialId,
        window: Duration,
    ) -> Result<u32, RepoError> {
        let guard = self.sentinel_events.lock();
        let window = chrono::Duration::from_std(window).map_err(|_| RepoError::InvalidState)?;
        let window_start = self.clock.now() - window;
        let count = guard
            .iter()
            .filter(|row| row.credential_id == *credential_id && row.detected_at > window_start)
            .count();
        // u32 is plenty — even at one sentinel event per second, 1h
        // window caps at 3600.
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }
}

#[async_trait::async_trait]
impl RefreshClaimAdjudicator for InMemoryRefreshClaimRepo {
    async fn adjudicate(
        &self,
        credential_id: &CredentialId,
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
            .get(credential_id)
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
                .filter(|event| event.credential_id == *credential_id)
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
            claims.remove(credential_id);
            return Ok(recorded_adjudication);
        }

        claims.remove(credential_id);
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
                    credential_id: *credential_id,
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
mod tests {
    use super::*;

    /// A clock this module moves by hand, so "expired" and "an hour ago" cost
    /// no sleeping. The adapter's own clock is the seam; nothing else in the
    /// claim path reads time.
    struct ManualClock {
        now: Mutex<DateTime<Utc>>,
    }

    impl ManualClock {
        fn starting_now() -> Arc<Self> {
            Arc::new(Self {
                now: Mutex::new(Utc::now()),
            })
        }

        fn advance(&self, by: chrono::Duration) {
            *self.now.lock() += by;
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> DateTime<Utc> {
            *self.now.lock()
        }

        fn monotonic(&self) -> std::time::Instant {
            std::time::Instant::now()
        }
    }

    async fn acquired_claim(
        repo: &InMemoryRefreshClaimRepo,
        credential_id: &CredentialId,
    ) -> RefreshClaim {
        match repo
            .try_claim(
                credential_id,
                &ReplicaId::new("original-holder"),
                Duration::from_secs(30),
            )
            .await
            .expect("initial claim")
        {
            ClaimAttempt::Acquired(claim) => claim,
            ClaimAttempt::Contended { .. } => panic!("fresh credential must be claimable"),
            ClaimAttempt::OutcomeUnknown { .. } => panic!("fresh claim cannot be poisoned"),
        }
    }

    fn expire_claim(repo: &InMemoryRefreshClaimRepo, credential_id: &CredentialId) {
        let mut guard = repo.inner.lock();
        let row = guard
            .get_mut(credential_id)
            .expect("acquired claim row must exist");
        row.expires_at = repo.clock.now() - chrono::Duration::seconds(1);
    }

    #[tokio::test]
    async fn expired_claim_cannot_be_marked_in_flight() {
        let repo = InMemoryRefreshClaimRepo::new();
        let credential_id = CredentialId::new();
        let claim = acquired_claim(&repo, &credential_id).await;
        expire_claim(&repo, &credential_id);

        let error = repo
            .mark_sentinel(&claim.token)
            .await
            .expect_err("an expired claim must not authorize provider egress");

        assert!(matches!(error, RepoError::InvalidState));
        assert_eq!(
            repo.inner
                .lock()
                .get(&credential_id)
                .expect("rejected mark must preserve the claim row")
                .sentinel,
            SentinelState::Normal,
            "a rejected mark must not mutate sentinel state"
        );
    }

    #[tokio::test]
    async fn expired_in_flight_claim_is_preserved_until_reclaim() {
        let repo = InMemoryRefreshClaimRepo::new();
        let credential_id = CredentialId::new();
        let claim = acquired_claim(&repo, &credential_id).await;
        repo.mark_sentinel(&claim.token)
            .await
            .expect("live holder may mark provider egress");
        expire_claim(&repo, &credential_id);

        let attempt = repo
            .try_claim(
                &credential_id,
                &ReplicaId::new("challenger"),
                Duration::from_secs(30),
            )
            .await
            .expect("poisoned acquisition");
        assert!(
            matches!(attempt, ClaimAttempt::OutcomeUnknown { .. }),
            "try_claim must fail closed without erasing in-flight evidence"
        );
        let repeated = repo
            .try_claim(
                &credential_id,
                &ReplicaId::new("second-challenger"),
                Duration::from_secs(30),
            )
            .await
            .expect("repeated poisoned acquisition");
        assert!(matches!(repeated, ClaimAttempt::OutcomeUnknown { .. }));

        let reclaimed = repo.reclaim_stuck().await.expect("reclaim expired claim");
        assert_eq!(reclaimed.len(), 1);
        assert!(matches!(
            &reclaimed[0],
            ExpiredClaim::OutcomeUnknownAccounted {
                credential_id: accounted_id,
                previous_holder,
                previous_generation: 0,
            } if *accounted_id == credential_id
                && *previous_holder == ReplicaId::new("original-holder")
        ));
        let recorded = repo
            .count_sentinel_events_in_window(&credential_id, Duration::from_mins(1))
            .await
            .expect("count atomically recorded evidence");
        assert_eq!(
            recorded, 1,
            "reclaim must durably account in-flight evidence while retaining poison"
        );

        let next = repo
            .try_claim(
                &credential_id,
                &ReplicaId::new("challenger"),
                Duration::from_secs(30),
            )
            .await
            .expect("poisoned claim result");
        assert!(
            matches!(next, ClaimAttempt::OutcomeUnknown { .. }),
            "accounting must not release an unknown provider outcome"
        );
        assert!(
            repo.reclaim_stuck()
                .await
                .expect("idempotent poison accounting")
                .is_empty(),
            "the retained poison event must be accounted exactly once"
        );
    }

    #[tokio::test]
    async fn expired_normal_claim_can_be_taken_over_in_place() {
        let repo = InMemoryRefreshClaimRepo::new();
        let credential_id = CredentialId::new();
        let first = acquired_claim(&repo, &credential_id).await;
        expire_claim(&repo, &credential_id);

        let second = repo
            .try_claim(
                &credential_id,
                &ReplicaId::new("challenger"),
                Duration::from_secs(30),
            )
            .await
            .expect("expired normal takeover");
        let ClaimAttempt::Acquired(second) = second else {
            panic!("expired normal claim must remain directly reclaimable");
        };

        assert_eq!(second.token.generation, first.token.generation + 1);
    }

    #[tokio::test]
    async fn exact_confirmed_release_clears_expired_in_flight_claim() {
        let repo = InMemoryRefreshClaimRepo::new();
        let credential_id = CredentialId::new();
        let claim = acquired_claim(&repo, &credential_id).await;
        repo.mark_sentinel(&claim.token)
            .await
            .expect("mark provider boundary");
        expire_claim(&repo, &credential_id);

        repo.release(claim.token)
            .await
            .expect("exact confirmed finalization");
        let next = repo
            .try_claim(
                &credential_id,
                &ReplicaId::new("next-holder"),
                Duration::from_secs(30),
            )
            .await
            .expect("claim after exact finalization");
        assert!(
            matches!(next, ClaimAttempt::Acquired(_)),
            "exact confirmed finalization must not leave false poison"
        );
    }

    #[tokio::test]
    async fn old_generation_zero_evidence_does_not_mask_a_new_claim_lifecycle() {
        let repo = InMemoryRefreshClaimRepo::new();
        let credential_id = CredentialId::new();

        let first = acquired_claim(&repo, &credential_id).await;
        repo.mark_sentinel(&first.token)
            .await
            .expect("mark first provider boundary");
        expire_claim(&repo, &credential_id);
        assert_eq!(
            repo.reclaim_stuck()
                .await
                .expect("account first poison")
                .len(),
            1
        );
        // Reconciliation, not release, is how a poisoned lifecycle ends: the
        // incident the sweep just recorded must be resolved before the row can
        // go away, and `release` would now refuse it.
        repo.adjudicate(
            &credential_id,
            RefreshOutcomeDecision::ProviderNotApplied,
            "operator confirmed the first provider call never landed",
        )
        .await
        .expect("reconcile the first lifecycle's unknown outcome");

        let second = acquired_claim(&repo, &credential_id).await;
        assert_eq!(
            second.token.generation, 0,
            "a new row demonstrates why generation alone is not event identity"
        );
        repo.mark_sentinel(&second.token)
            .await
            .expect("mark second provider boundary");
        expire_claim(&repo, &credential_id);

        assert_eq!(
            repo.reclaim_stuck()
                .await
                .expect("account second poison")
                .len(),
            1,
            "evidence from the prior row lifecycle must not suppress new poison"
        );
        assert_eq!(
            repo.count_sentinel_events_in_window(&credential_id, Duration::from_mins(1))
                .await
                .expect("count both lifecycle events"),
            2
        );
    }

    #[tokio::test]
    async fn sentinel_window_excludes_evidence_older_than_the_window() -> Result<(), RepoError> {
        let clock = ManualClock::starting_now();
        let repo = InMemoryRefreshClaimRepo::with_clock(Arc::clone(&clock) as Arc<dyn Clock>);
        let credential_id = CredentialId::new();

        let claim = acquired_claim(&repo, &credential_id).await;
        repo.mark_sentinel(&claim.token)
            .await
            .expect("mark provider boundary");
        clock.advance(chrono::Duration::seconds(31));
        assert_eq!(
            repo.reclaim_stuck()
                .await
                .expect("account the in-flight expiry")
                .len(),
            1
        );

        let window = Duration::from_hours(24);
        assert_eq!(
            repo.count_sentinel_events_in_window(&credential_id, window)
                .await?,
            1,
            "the accounted incident is inside the window"
        );
        clock.advance(chrono::Duration::hours(25));
        assert_eq!(
            repo.count_sentinel_events_in_window(&credential_id, window)
                .await?,
            0,
            "events before the window must be excluded"
        );
        Ok(())
    }
}
