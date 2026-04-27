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
    ClaimAttempt, ClaimToken, HeartbeatError, ReclaimedClaim, RefreshClaim, RefreshClaimRepo,
    ReplicaId, RepoError, SentinelState,
};

#[derive(Clone, Debug)]
struct ClaimRow {
    claim_id: Uuid,
    generation: u64,
    holder: ReplicaId,
    #[allow(dead_code, reason = "kept for future event/metric emission")]
    acquired_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    sentinel: SentinelState,
}

/// One sentinel event record kept in the in-memory ring.
#[derive(Clone, Debug)]
struct SentinelEventRow {
    credential_id: CredentialId,
    detected_at: DateTime<Utc>,
    #[allow(
        dead_code,
        reason = "kept for symmetry with the SQL backends; surfaces in diagnostics"
    )]
    crashed_holder: ReplicaId,
    #[allow(
        dead_code,
        reason = "kept for symmetry with the SQL backends; surfaces in diagnostics"
    )]
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
    /// clock. Gated behind `#[cfg(any(test, feature = "test-util"))]` so
    /// production builds cannot construct events out-of-band.
    #[cfg(any(test, feature = "test-util"))]
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
        let now = Utc::now();
        let mut guard = self.inner.lock();

        if let Some(existing) = guard.get(credential_id)
            && existing.expires_at > now
        {
            return Ok(ClaimAttempt::Contended {
                existing_expires_at: existing.expires_at,
            });
        }
        // No row OR existing expired — claim wins. Generation bumps if
        // we're overwriting.

        let claim_id = Uuid::new_v4();
        let generation = guard.get(credential_id).map_or(0, |row| row.generation + 1);
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
            // Resetting `sentinel` to `Normal` on overwrite is intentional.
            // Sub-spec §3.5 invariant `reclaim_sweep_interval ≤ claim_ttl`
            // (validated in `RefreshCoordConfig::validate`) guarantees the
            // reclaim sweep observes any expired sentinel state before
            // this overwrite path runs — so dropping the previous holder's
            // `RefreshInFlight` here cannot cause silent loss of a
            // sentinel signal that the sweep would otherwise have
            // recorded.
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
        let now = Utc::now();
        let extension = chrono::Duration::from_std(ttl).map_err(|e| {
            HeartbeatError::Repo(RepoError::InvalidState(format!("invalid ttl: {e}")))
        })?;
        let mut guard = self.inner.lock();

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
        let row = guard
            .values_mut()
            .find(|r| r.claim_id == token.claim_id && r.generation == token.generation);
        // Mirrors heartbeat's claim-loss check: a missing row means the
        // claim was reclaimed or released and another replica owns the
        // credential. Silently succeeding would let the holder proceed to
        // the IdP POST while another replica already owns the row.
        match row {
            Some(r) => {
                r.sentinel = SentinelState::RefreshInFlight;
                Ok(())
            },
            None => Err(RepoError::InvalidState(
                "mark_sentinel: claim lost — token no longer owns the row".to_string(),
            )),
        }
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError> {
        let now = Utc::now();
        let mut guard = self.inner.lock();
        let mut out = Vec::new();

        let stuck: Vec<CredentialId> = guard
            .iter()
            .filter(|(_, r)| r.expires_at < now)
            .map(|(k, _)| *k)
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

    async fn record_sentinel_event(
        &self,
        credential_id: &CredentialId,
        crashed_holder: &ReplicaId,
        generation: u64,
    ) -> Result<(), RepoError> {
        let mut guard = self.sentinel_events.lock();
        // Bound the in-memory event log: drop entries older than the §3.4
        // retention horizon (24h, generously above the default 1h rolling
        // window) on every insert. Keeps memory and the
        // `count_sentinel_events_in_window` scan O(events-in-24h) regardless
        // of process uptime. SQL backends are bounded by their tables and
        // external GC, so this prune lives only on the in-memory impl.
        let cutoff = Utc::now() - chrono::Duration::hours(24);
        guard.retain(|row| row.detected_at > cutoff);
        guard.push(SentinelEventRow {
            credential_id: *credential_id,
            detected_at: Utc::now(),
            crashed_holder: crashed_holder.clone(),
            generation,
        });
        Ok(())
    }

    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &CredentialId,
        window_start: DateTime<Utc>,
    ) -> Result<u32, RepoError> {
        let guard = self.sentinel_events.lock();
        let count = guard
            .iter()
            .filter(|row| row.credential_id == *credential_id && row.detected_at > window_start)
            .count();
        // u32 is plenty — even at one sentinel event per second, 1h
        // window caps at 3600.
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }
}
