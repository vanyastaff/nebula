//! Refresh-claim conformance for the in-memory reference model.
//!
//! The in-memory adapter is never a deployment target; running the shared oracle
//! against it keeps the reference model and the two SQL deployment backends
//! answering identically, so a divergence shows up as the same named case
//! failing on one of the three.
//!
//! This fixture owns a hand-moved clock, and that is the point: expiry is a
//! claim about liveness, the SQL backends decide it in the database, and the
//! in-memory adapter is the only backend where a test can reach it. Everything
//! the fixture does with that clock stays behind the shared trait, so no case
//! knows which backend it is running on.
//!
//! One case is declared skipped here, as an `#[ignore]`d test so the run's
//! printed denominator shows it:
//! `a_failed_decision_write_leaves_the_claim_poisoned_and_unrecorded` needs a
//! resolution write to fail after the claim row has been deleted, which the
//! in-memory adapter can only be made to do by growing a production failure
//! seam. This runner therefore asserts **29** of the shared 30 cases; sqlite
//! asserts all 30, and postgres asserts all 30 only when it can reach a database.
//!
//! Seen from the variant side, the same gap: this adapter emits two of the five
//! `RefreshClaimAdjudicationError` variants at its own sites — `NotPoisoned` and
//! `EvidenceConflict`. `Storage` and `AcknowledgementUnknown` are emitted only by
//! the SQL adapters, so the 503 path behind the `Storage` arm is unexercised
//! here. (`InvalidEvidence` is raised by the shared evidence check in
//! `refresh_claim/mod.rs`, so all three adapters emit that one.)

#[macro_use]
#[path = "support/refresh_claim_oracle.rs"]
mod oracle;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use nebula_core::accessor::Clock;
use nebula_storage::credential::refresh_claim::{
    RefreshAdjudication, RefreshClaimAdjudicationError, RefreshClaimAdjudicator,
    RefreshOutcomeDecision,
};
use nebula_storage::credential::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, InMemoryRefreshClaimRepo,
    RefreshClaimRepo, ReplicaId, RepoError,
};
use uuid::Uuid;

/// A clock the fixture moves by hand, so "expired" and "two minutes ago" cost
/// no sleeping. The adapter's own clock is the seam; nothing else in the claim
/// path reads time.
struct ManualClock {
    now: Mutex<DateTime<Utc>>,
}

impl ManualClock {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(Utc::now()),
        })
    }

    fn advance(&self, by: chrono::Duration) {
        *self.now.lock().expect("the manual clock is never poisoned") += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.now.lock().expect("the manual clock is never poisoned")
    }

    fn monotonic(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
}

/// What the fixture has observed about one credential's claim row.
///
/// The adapter keeps its rows private, so the fixture derives the poison
/// predicate from the lifecycles it drives: a row the fixture marked
/// provider-in-flight and then expired is exactly the row `try_claim` answers
/// `OutcomeUnknown` with, and the adapter deletes it only where the fixture can
/// see the authority that deleted it — adjudication, release, or a takeover.
#[derive(Clone, Copy)]
struct TrackedClaim {
    claim_id: Uuid,
    generation: u64,
    in_flight: bool,
    expired: bool,
}

impl TrackedClaim {
    /// Does `token` name this row?
    fn matches(&self, token: &ClaimToken) -> bool {
        self.claim_id == token.claim_id && self.generation == token.generation
    }
}

/// The in-memory adapter plus the clock and observation a shared case needs.
struct InMemoryRefreshClaimFixture {
    repo: InMemoryRefreshClaimRepo,
    clock: Arc<ManualClock>,
    namespace: String,
    tracked: Mutex<HashMap<CredentialId, TrackedClaim>>,
    resolved: Mutex<HashMap<CredentialId, u64>>,
}

impl InMemoryRefreshClaimFixture {
    fn new() -> Self {
        let clock = ManualClock::new();
        Self {
            repo: InMemoryRefreshClaimRepo::with_clock(Arc::clone(&clock) as Arc<dyn Clock>),
            clock,
            namespace: Uuid::new_v4().simple().to_string(),
            tracked: Mutex::new(HashMap::new()),
            resolved: Mutex::new(HashMap::new()),
        }
    }

    /// Mutate the tracked row that `token` names, if the fixture still holds
    /// one for it.
    ///
    /// The token is the only handle a case has on the row the adapter deleted
    /// or bumped, so every observation here is keyed by it.
    fn with_tracked(&self, token: &ClaimToken, update: impl FnOnce(&mut TrackedClaim)) {
        let mut tracked = self
            .tracked
            .lock()
            .expect("the tracking map is never poisoned");
        if let Some(entry) = tracked.values_mut().find(|entry| entry.matches(token)) {
            update(entry);
        }
    }
}

#[async_trait::async_trait]
impl RefreshClaimRepo for InMemoryRefreshClaimFixture {
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        let attempt = self.repo.try_claim(credential_id, holder, ttl).await?;
        if let ClaimAttempt::Acquired(claim) = &attempt {
            // An acquisition either creates a Normal row or overwrites an
            // expired Normal one, so the row it returns is never in flight and
            // never expired.
            self.tracked
                .lock()
                .expect("the tracking map is never poisoned")
                .insert(
                    *credential_id,
                    TrackedClaim {
                        claim_id: claim.token.claim_id,
                        generation: claim.token.generation,
                        in_flight: false,
                        expired: false,
                    },
                );
        }
        Ok(attempt)
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        let result = self.repo.heartbeat(token, ttl).await;
        if result.is_ok() {
            // The adapter refuses to extend an expired claim, so a successful
            // heartbeat is also proof the row moved back inside its lease.
            self.with_tracked(token, |entry| entry.expired = false);
        }
        result
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError> {
        self.repo.release(token.clone()).await?;
        // The adapter deletes by (claim id, generation); a superseded or
        // forged generation leaves the current row alone, and so does this. A
        // refused release never reaches here, which is what keeps the refused
        // claim's poison observable.
        self.tracked
            .lock()
            .expect("the tracking map is never poisoned")
            .retain(|_, entry| !entry.matches(&token));
        Ok(())
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        let result = self.repo.mark_sentinel(token).await;
        if result.is_ok() {
            self.with_tracked(token, |entry| entry.in_flight = true);
        }
        result
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ExpiredClaim>, RepoError> {
        let outcomes = self.repo.reclaim_stuck().await?;
        for outcome in &outcomes {
            if let ExpiredClaim::ReclaimedNormal { credential_id, .. } = outcome {
                self.tracked
                    .lock()
                    .expect("the tracking map is never poisoned")
                    .remove(credential_id);
            }
        }
        Ok(outcomes)
    }

    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &CredentialId,
        window: Duration,
    ) -> Result<u32, RepoError> {
        self.repo
            .count_sentinel_events_in_window(credential_id, window)
            .await
    }
}

#[async_trait::async_trait]
impl RefreshClaimAdjudicator for InMemoryRefreshClaimFixture {
    async fn adjudicate(
        &self,
        credential_id: &CredentialId,
        decision: RefreshOutcomeDecision,
        evidence: &str,
    ) -> Result<RefreshAdjudication, RefreshClaimAdjudicationError> {
        let adjudication = self
            .repo
            .adjudicate(credential_id, decision, evidence)
            .await?;
        if adjudication.changed {
            // This is bookkeeping, and the in-memory fixture is the only backend
            // where it has to be: the adapter's resolutions live in private
            // state it exposes no read for, so the fixture counts its own
            // `changed` answers and `resolved_incident_count` reports that tally
            // rather than adapter state. `incident_count` is not a cross-check
            // on it — the sweep's count is resolution-blind by design — so a
            // case that means to observe a *recorded resolution* on this backend
            // cannot, and the SQL fixtures are where that read exists.
            *self
                .resolved
                .lock()
                .expect("the resolution map is never poisoned")
                .entry(*credential_id)
                .or_insert(0) += 1;
            self.tracked
                .lock()
                .expect("the tracking map is never poisoned")
                .remove(credential_id);
        }
        Ok(adjudication)
    }
}

#[async_trait::async_trait]
impl oracle::RefreshClaimFixture for InMemoryRefreshClaimFixture {
    fn credential(&self, case: &str) -> CredentialId {
        oracle::case_credential(&self.namespace, case)
    }

    fn replica(&self, seed: u8) -> ReplicaId {
        ReplicaId::new(format!("replica-{seed:02x}"))
    }

    fn claim_ttl(&self) -> Duration {
        // Small because the fixture expires claims by advancing the shared
        // clock: the production 30-second lease would make every expiry case
        // time-travel half a minute and drag its incidents out of the windows
        // the sentinel cases assert on.
        Duration::from_secs(1)
    }

    async fn expire_claims(&self, credential: &CredentialId) {
        self.clock.advance(chrono::Duration::seconds(2));
        if let Some(entry) = self
            .tracked
            .lock()
            .expect("the tracking map is never poisoned")
            .get_mut(credential)
        {
            entry.expired = true;
        }
    }

    async fn seed_unresolved_incidents(&self, credential: &CredentialId, count: u32) {
        oracle::replay_poisoned_lifecycles(self, credential, count).await;
    }

    async fn incident_count(&self, credential: &CredentialId) -> u64 {
        u64::from(
            self.repo
                .count_sentinel_events_in_window(credential, oracle::INCIDENT_WINDOW)
                .await
                .expect("the sentinel count must be readable"),
        )
    }

    async fn resolved_incident_count(&self, credential: &CredentialId) -> u64 {
        self.resolved
            .lock()
            .expect("the resolution map is never poisoned")
            .get(credential)
            .copied()
            .unwrap_or(0)
    }

    async fn age_incidents(&self, _credential: &CredentialId, by: Duration) {
        // One process, one clock: moving it forward is the same statement as
        // pushing this credential's incidents backwards.
        self.clock
            .advance(chrono::Duration::from_std(by).expect("age fits a chrono duration"));
    }

    async fn inject_decision_write_failure(&self, credential: &CredentialId) {
        panic!(
            "the in-memory adapter cannot fail a resolution write after the claim-row delete for \
             {credential}: it has no failure seam to inject one at, which is why the case needing \
             that failure is `#[ignore]`d in this runner"
        );
    }

    async fn clear_decision_write_failure(&self, credential: &CredentialId) {
        panic!(
            "no decision-write failure was ever injected for {credential}: the in-memory adapter \
             has no failure seam to install one at"
        );
    }

    async fn poisoned_claim_exists(&self, credential: &CredentialId) -> bool {
        self.tracked
            .lock()
            .expect("the tracking map is never poisoned")
            .get(credential)
            .is_some_and(|entry| entry.in_flight && entry.expired)
    }
}

async fn fixture() -> Option<InMemoryRefreshClaimFixture> {
    Some(InMemoryRefreshClaimFixture::new())
}

refresh_claim_conformance_suite!(fixture());

// Declared skipped, not silently passed: this backend marks the generated test
// ignored, so nextest reports it in the denominator rather than counting it as a
// pass. The case body asserts unconditionally, so a run that opts into ignored
// tests fails loudly here instead of passing with nothing checked. This runner
// asserts 29 of the 30 shared cases.
refresh_claim_case!(
    #[ignore = "the in-memory adapter cannot fail a resolution write after the claim-row delete without growing a production failure seam"]
    a_failed_decision_write_leaves_the_claim_poisoned_and_unrecorded,
    0x2D,
    fixture()
);
