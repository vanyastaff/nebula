//! Test fixtures for `RefreshCoordinator` integration tests.
//!
//! These fixtures wrap a `RefreshClaimRepo` and override individual
//! methods to model failure modes the coordinator must handle (release
//! errors, heartbeat loss, persistent contention). Extracted from
//! `coordinator.rs::tests` so the test module itself stays focused on
//! assertions rather than trait-forwarding boilerplate.
//!
//! All three fixtures forward most methods to an inner repo; the
//! `inner` field is `Arc<dyn RefreshClaimRepo>` for those that need a
//! live in-memory backing, and absent for `AlwaysContendedRepo` which
//! never reads inner state.

#![cfg(test)]

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use nebula_core::CredentialId;
use nebula_storage::credential::{
    ClaimAttempt, ClaimToken, HeartbeatError, ReclaimedClaim, RefreshClaimRepo, ReplicaId,
    RepoError,
};

// ──────────────────────────────────────────────────────────────────────────
// FlakyReleaseRepo — forwards everything except `release`, which always
// fails. Used to prove the coordinator does not mask a successful refresh
// result when release fails after the user closure already returned `Ok`.
// ──────────────────────────────────────────────────────────────────────────

pub(crate) struct FlakyReleaseRepo {
    pub inner: Arc<dyn RefreshClaimRepo>,
}

#[async_trait::async_trait]
impl RefreshClaimRepo for FlakyReleaseRepo {
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        self.inner.try_claim(credential_id, holder, ttl).await
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        self.inner.heartbeat(token, ttl).await
    }

    async fn release(&self, _token: ClaimToken) -> Result<(), RepoError> {
        Err(RepoError::InvalidState("simulated release failure".into()))
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        self.inner.mark_sentinel(token).await
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError> {
        self.inner.reclaim_stuck().await
    }

    async fn record_sentinel_event(
        &self,
        credential_id: &CredentialId,
        crashed_holder: &ReplicaId,
        generation: u64,
    ) -> Result<(), RepoError> {
        self.inner
            .record_sentinel_event(credential_id, crashed_holder, generation)
            .await
    }

    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &CredentialId,
        window_start: DateTime<Utc>,
    ) -> Result<u32, RepoError> {
        self.inner
            .count_sentinel_events_in_window(credential_id, window_start)
            .await
    }
}

// ──────────────────────────────────────────────────────────────────────────
// AlwaysContendedRepo — every `try_claim` returns `Contended` with a short
// `existing_expires_at` so the backoff loop completes quickly. Used by the
// `ContentionExhausted` exhaustion test (m2 wave-3).
// ──────────────────────────────────────────────────────────────────────────

pub(crate) struct AlwaysContendedRepo;

#[async_trait::async_trait]
impl RefreshClaimRepo for AlwaysContendedRepo {
    async fn try_claim(
        &self,
        _credential_id: &CredentialId,
        _holder: &ReplicaId,
        _ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        Ok(ClaimAttempt::Contended {
            existing_expires_at: Utc::now() + chrono::Duration::milliseconds(50),
        })
    }

    async fn heartbeat(&self, _token: &ClaimToken, _ttl: Duration) -> Result<(), HeartbeatError> {
        Ok(())
    }

    async fn release(&self, _token: ClaimToken) -> Result<(), RepoError> {
        Ok(())
    }

    async fn mark_sentinel(&self, _token: &ClaimToken) -> Result<(), RepoError> {
        Ok(())
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError> {
        Ok(Vec::new())
    }

    async fn record_sentinel_event(
        &self,
        _credential_id: &CredentialId,
        _crashed_holder: &ReplicaId,
        _generation: u64,
    ) -> Result<(), RepoError> {
        Ok(())
    }

    async fn count_sentinel_events_in_window(
        &self,
        _credential_id: &CredentialId,
        _window_start: DateTime<Utc>,
    ) -> Result<u32, RepoError> {
        Ok(0)
    }
}

// ──────────────────────────────────────────────────────────────────────────
// AlwaysFailHeartbeatRepo — forwards everything except `heartbeat`, which
// always returns `HeartbeatError::ClaimLost`. Used to prove the heartbeat
// loss cancels the concurrent `do_refresh` closure (M1 wave-3).
// ──────────────────────────────────────────────────────────────────────────

pub(crate) struct AlwaysFailHeartbeatRepo {
    pub inner: Arc<dyn RefreshClaimRepo>,
}

#[async_trait::async_trait]
impl RefreshClaimRepo for AlwaysFailHeartbeatRepo {
    async fn try_claim(
        &self,
        credential_id: &CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        self.inner.try_claim(credential_id, holder, ttl).await
    }

    async fn heartbeat(&self, _token: &ClaimToken, _ttl: Duration) -> Result<(), HeartbeatError> {
        Err(HeartbeatError::ClaimLost)
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError> {
        self.inner.release(token).await
    }

    async fn mark_sentinel(&self, token: &ClaimToken) -> Result<(), RepoError> {
        self.inner.mark_sentinel(token).await
    }

    async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RepoError> {
        self.inner.reclaim_stuck().await
    }

    async fn record_sentinel_event(
        &self,
        credential_id: &CredentialId,
        crashed_holder: &ReplicaId,
        generation: u64,
    ) -> Result<(), RepoError> {
        self.inner
            .record_sentinel_event(credential_id, crashed_holder, generation)
            .await
    }

    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &CredentialId,
        window_start: DateTime<Utc>,
    ) -> Result<u32, RepoError> {
        self.inner
            .count_sentinel_events_in_window(credential_id, window_start)
            .await
    }
}
