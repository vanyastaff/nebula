//! End-to-end exercise of the engine-side lease lifecycle (ADR-0051 Phase D).
//!
//! Demonstrates the canonical wiring pattern:
//!
//! 1. The engine spawns a [`LeaseLifecycle`] with a `CancellationToken`.
//! 2. A composition root resolves a credential through a leased provider
//!    and registers the issued lease with `track`.
//! 3. The scheduler proactively renews at 70% of the TTL — verified
//!    through a `tokio::time::pause` clock.
//! 4. On credential rotation commit the orchestrator calls
//!    [`LeaseLifecycle::revoke_for_credential`] — the lease is torn down
//!    upstream and removed from the registry.
//!
//! The mock provider is a minimal `LeasedProvider` that counts renew /
//! revoke calls — sufficient to assert the lifecycle is doing what it
//! says without bringing in a real backend.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use nebula_credential::{
    CredentialId, ExternalProvider, ExternalReference, LeaseHandle, LeasedProvider, ProviderError,
    ProviderFuture, ProviderResolution, SecretString,
};
use nebula_engine::credential::{LeaseLifecycle, LeaseLifecycleConfig};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default)]
struct RenewRevokeProvider {
    renew_calls: AtomicU32,
    revoke_calls: AtomicU32,
    ttl_secs: u32,
}

impl RenewRevokeProvider {
    fn new(ttl_secs: u32) -> Arc<Self> {
        Arc::new(Self {
            renew_calls: AtomicU32::new(0),
            revoke_calls: AtomicU32::new(0),
            ttl_secs,
        })
    }
}

impl ExternalProvider for RenewRevokeProvider {
    fn resolve<'a>(&'a self, _r: &'a ExternalReference) -> ProviderFuture<'a> {
        ProviderFuture::ready(Ok(ProviderResolution::from_secret(SecretString::new(
            "irrelevant",
        ))))
    }
    fn provider_name(&self) -> &'static str {
        "mock-leased"
    }
    fn lease_renewal(&self) -> Option<&dyn LeasedProvider> {
        Some(self)
    }
}

impl LeasedProvider for RenewRevokeProvider {
    fn renew<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        self.renew_calls.fetch_add(1, Ordering::SeqCst);
        let refreshed = LeaseHandle::new(
            Cow::Borrowed("mock-leased"),
            lease.lease_id.clone(),
            chrono::Utc::now(),
            Duration::from_secs(u64::from(self.ttl_secs)),
        );
        ProviderFuture::ready(Ok(ProviderResolution::with_lease(
            SecretString::new("renewed"),
            refreshed,
        )))
    }
    fn revoke<'a>(&'a self, _lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        self.revoke_calls.fetch_add(1, Ordering::SeqCst);
        ProviderFuture::ready(Ok(ProviderResolution::empty()))
    }
}

#[derive(Debug)]
struct UnavailableProvider;

impl ExternalProvider for UnavailableProvider {
    fn resolve<'a>(&'a self, _r: &'a ExternalReference) -> ProviderFuture<'a> {
        ProviderFuture::ready(Err(ProviderError::Unavailable {
            reason: "test".to_owned(),
        }))
    }
    fn provider_name(&self) -> &'static str {
        "mock-unavailable"
    }
    fn lease_renewal(&self) -> Option<&dyn LeasedProvider> {
        Some(self)
    }
}

impl LeasedProvider for UnavailableProvider {
    fn renew<'a>(&'a self, _lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        ProviderFuture::ready(Err(ProviderError::Unavailable {
            reason: "test".to_owned(),
        }))
    }
    fn revoke<'a>(&'a self, _lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        ProviderFuture::ready(Err(ProviderError::Unavailable {
            reason: "test".to_owned(),
        }))
    }
}

fn lease_resolution(provider: &str, lease_id: &str, ttl_secs: u64) -> ProviderResolution {
    let lease = LeaseHandle::new(
        Cow::Owned(provider.to_owned()),
        lease_id,
        chrono::Utc::now(),
        Duration::from_secs(ttl_secs),
    );
    ProviderResolution::with_lease(SecretString::new("secret"), lease)
}

#[tokio::test(start_paused = true)]
async fn lifecycle_renews_then_rotation_revokes_for_credential() {
    let shutdown = CancellationToken::new();
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let provider = RenewRevokeProvider::new(200);
    let credential_id = CredentialId::new();

    // Step 1: register a Vault-style dynamic lease.
    let _token = lifecycle
        .track(
            Arc::clone(&provider) as Arc<dyn LeasedProvider>,
            lease_resolution("mock-leased", "lease-1", 200),
            Some(credential_id),
        )
        .await
        .expect("track succeeds");
    assert_eq!(lifecycle.active_lease_count().await, 1);

    // Step 2: advance past 70% of TTL — scheduler renews automatically.
    tokio::time::advance(Duration::from_secs(141)).await;
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(provider.renew_calls.load(Ordering::SeqCst), 1);

    // Step 3: rotation commits — the orchestrator calls
    // `revoke_for_credential`. Mirrors the wiring point documented on
    // `RotationTransaction::mark_committed`.
    let revoked = lifecycle.revoke_for_credential(credential_id).await;
    assert_eq!(revoked, 1);
    assert_eq!(provider.revoke_calls.load(Ordering::SeqCst), 1);
    assert_eq!(lifecycle.active_lease_count().await, 0);

    shutdown.cancel();
}

#[tokio::test(start_paused = true)]
async fn revoke_for_credential_with_provider_failure_does_not_block_rotation() {
    // Revoke-on-rotate is best-effort cleanup: if the provider is
    // unavailable, the rotation pipeline MUST still proceed. The lease
    // is removed from the registry regardless.
    let shutdown = CancellationToken::new();
    let lifecycle = LeaseLifecycle::spawn(
        LeaseLifecycleConfig::default(),
        None,
        None,
        shutdown.clone(),
    );
    let credential_id = CredentialId::new();

    lifecycle
        .track(
            Arc::new(UnavailableProvider) as Arc<dyn LeasedProvider>,
            lease_resolution("mock-unavailable", "lease-flaky", 600),
            Some(credential_id),
        )
        .await
        .expect("track succeeds");

    // Provider revoke will fail with Unavailable; the helper returns 0
    // (no successful revokes) but does not propagate the error. The
    // lease is dropped from the registry either way.
    let revoked = lifecycle.revoke_for_credential(credential_id).await;
    assert_eq!(revoked, 0);
    assert_eq!(lifecycle.active_lease_count().await, 0);

    shutdown.cancel();
}
