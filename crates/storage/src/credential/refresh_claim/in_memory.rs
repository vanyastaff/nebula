//! In-memory `RefreshClaimRepo` impl for tests + desktop-mode fallback.
//!
//! Single-process scope — no cross-replica coordination. CAS uses a
//! `parking_lot::Mutex` over `HashMap<CredentialId, ClaimRow>`.

use std::{collections::HashMap, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use parking_lot::Mutex;
use uuid::Uuid;

use super::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, RefreshClaim, RefreshClaimRepo,
    ReplicaId, RepoError, SentinelState,
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
}

/// In-memory `RefreshClaimRepo`. Cheap to clone (Arc-backed inner).
#[derive(Clone, Default)]
pub struct InMemoryRefreshClaimRepo {
    inner: Arc<Mutex<HashMap<CredentialId, ClaimRow>>>,
    sentinel_events: Arc<Mutex<Vec<SentinelEventRow>>>,
}

impl InMemoryRefreshClaimRepo {
    /// Create a fresh, empty repo.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Test-only: push a synthetic sentinel event with a caller-supplied
    /// `detected_at`. Used by the in-memory smoke tests to seed
    /// pre-prune-cutoff entries (24h-old) without manipulating the system
    /// clock. Gated behind `#[cfg(test)]` so production builds cannot
    /// construct events out-of-band.
    #[cfg(test)]
    pub fn push_sentinel_event_at(
        &self,
        credential_id: &CredentialId,
        crashed_holder: &ReplicaId,
        generation: u64,
        detected_at: DateTime<Utc>,
    ) {
        let mut guard = self.sentinel_events.lock();
        guard.push(SentinelEventRow {
            credential_id: *credential_id,
            claim_id: Uuid::new_v4(),
            detected_at,
            crashed_holder: crashed_holder.clone(),
            generation,
        });
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
        let now = Utc::now();

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
        let now = Utc::now();

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
        let mut guard = self.inner.lock();
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
        let now = Utc::now();
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
        let now = Utc::now();
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
        let window_start = Utc::now() - window;
        let count = guard
            .iter()
            .filter(|row| row.credential_id == *credential_id && row.detected_at > window_start)
            .count();
        // u32 is plenty — even at one sentinel event per second, 1h
        // window caps at 3600.
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        row.expires_at = Utc::now() - chrono::Duration::seconds(1);
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
        repo.release(first.token)
            .await
            .expect("exactly finalize first lifecycle");

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
    async fn sentinel_window_excludes_old_test_evidence() -> Result<(), RepoError> {
        let repo = InMemoryRefreshClaimRepo::new();
        let stale_cid = CredentialId::new();
        let holder = ReplicaId::new("replica-A");

        let stale_timestamp = Utc::now() - chrono::Duration::hours(25);
        repo.push_sentinel_event_at(&stale_cid, &holder, 1, stale_timestamp);

        let count = repo
            .count_sentinel_events_in_window(&stale_cid, Duration::from_hours(24))
            .await?;
        assert_eq!(count, 0, "events before the window must be excluded");
        Ok(())
    }
}
