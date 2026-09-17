use std::sync::Arc;

use super::*;
use crate::{
    SecretString,
    provider::{ExternalProviderChain, ExternalReference, ProviderError, ProviderResolution},
};

// ────────────────────────────────────────────────────────────────────
// Mocks: a plain provider and a leased provider.
// ────────────────────────────────────────────────────────────────────

/// Plain `ExternalProvider` with no lease capability.
#[derive(Debug)]
struct PlainProvider {
    name: &'static str,
}

impl ExternalProvider for PlainProvider {
    fn resolve<'a>(&'a self, _reference: &'a ExternalReference) -> ProviderFuture<'a> {
        ProviderFuture::ready(Ok(ProviderResolution::from_secret(SecretString::new(
            "plain",
        ))))
    }

    fn provider_name(&self) -> &str {
        self.name
    }
}

/// Provider that supports the leased lifecycle.
#[derive(Debug)]
struct LeasedMock {
    name: &'static str,
}

impl ExternalProvider for LeasedMock {
    fn resolve<'a>(&'a self, _reference: &'a ExternalReference) -> ProviderFuture<'a> {
        ProviderFuture::ready(Ok(ProviderResolution::from_secret(SecretString::new(
            "leased-secret",
        ))))
    }

    fn provider_name(&self) -> &str {
        self.name
    }

    fn lease_renewal(&self) -> Option<&dyn LeasedProvider> {
        Some(self)
    }
}

impl LeasedProvider for LeasedMock {
    fn renew<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        // Defensively check attribution: a chain that misroutes should
        // surface as NotFound rather than acting on the wrong lease.
        if !self.handles_lease(lease) {
            return ProviderFuture::ready(Err(ProviderError::NotFound {
                path: format!(
                    "lease attributed to {:?}, but this provider is {:?}",
                    lease.provider, self.name
                ),
            }));
        }
        // Echo the lease id back as the "secret" so the test can assert
        // the call reached the right provider with the right lease.
        ProviderFuture::ready(Ok(ProviderResolution::from_secret(SecretString::new(
            format!("renewed:{}@{}", lease.lease_id, self.name),
        ))))
    }

    fn revoke<'a>(&'a self, lease: &'a LeaseHandle) -> ProviderFuture<'a> {
        if !self.handles_lease(lease) {
            return ProviderFuture::ready(Err(ProviderError::NotFound {
                path: format!(
                    "lease attributed to {:?}, but this provider is {:?}",
                    lease.provider, self.name
                ),
            }));
        }
        ProviderFuture::ready(Ok(ProviderResolution::from_secret(SecretString::new(
            format!("revoked:{}@{}", lease.lease_id, self.name),
        ))))
    }
}

fn sample_lease(provider: &'static str, id: &str) -> LeaseHandle {
    LeaseHandle::new(
        provider,
        id,
        chrono::Utc::now(),
        std::time::Duration::from_mins(1),
    )
}

// ────────────────────────────────────────────────────────────────────
// B4 — capability discovery + chain forwarding + round-trip.
// ────────────────────────────────────────────────────────────────────

#[test]
fn default_lease_renewal_returns_none_on_non_leased() {
    let p = PlainProvider { name: "plain" };
    assert!(
        p.lease_renewal().is_none(),
        "non-leased provider must advertise no lease capability via default"
    );
}

#[test]
fn explicit_impl_returns_some_self() {
    let p = LeasedMock { name: "vault-stub" };
    let view = p
        .lease_renewal()
        .expect("leased provider must advertise capability");
    // The capability view points back at the same provider — verifiable
    // through `provider_name`, which delegates via the supertrait.
    assert_eq!(view.provider_name(), "vault-stub");
}

#[test]
fn chain_lease_renewal_surfaces_self_when_any_child_is_leased() {
    // The chain becomes a `LeasedProvider` (it dispatches by
    // `handles_lease`), so `lease_renewal()` returns `Some(self)` —
    // the chain itself — rather than directly handing out the first
    // leased child. That keeps routing centralised in the chain.
    let chain = ExternalProviderChain::first_try(
        "plain",
        Arc::new(PlainProvider { name: "plain" }) as Arc<dyn ExternalProvider>,
    )
    .or_else(
        "leased",
        Arc::new(LeasedMock { name: "vault-stub" }) as Arc<dyn ExternalProvider>,
    )
    .or_else(
        "plain-2",
        Arc::new(PlainProvider { name: "plain-2" }) as Arc<dyn ExternalProvider>,
    );

    let view = chain
        .lease_renewal()
        .expect("chain with a leased child must advertise capability");
    // `provider_name` reports `"chain"` — the chain wears its own name
    // when acting as the lease dispatcher. Children are reached via
    // `handles_lease` lookup at renew/revoke time.
    assert_eq!(view.provider_name(), "chain");
    assert!(
        view.handles_lease(&sample_lease("vault-stub", "lease-abc")),
        "chain reports it can handle a lease attributed to its leased child"
    );
    assert!(
        !view.handles_lease(&sample_lease("unrelated", "lease-xyz")),
        "chain rejects leases attributed to providers not in the chain"
    );
}

#[test]
fn chain_lease_renewal_returns_none_when_no_child_leases() {
    let chain = ExternalProviderChain::first_try(
        "a",
        Arc::new(PlainProvider { name: "a" }) as Arc<dyn ExternalProvider>,
    )
    .or_else(
        "b",
        Arc::new(PlainProvider { name: "b" }) as Arc<dyn ExternalProvider>,
    );

    assert!(
        chain.lease_renewal().is_none(),
        "chain with no leased children must report no capability"
    );
}

#[tokio::test]
async fn chain_routes_renew_revoke_by_lease_attribution() {
    // Regression guard for the Codex P1 finding: a chain with multiple
    // leased children must dispatch renew/revoke to the provider that
    // actually issued the lease, not the first-encountered leased child.
    let chain = ExternalProviderChain::first_try(
        "first-leased",
        Arc::new(LeasedMock { name: "alpha" }) as Arc<dyn ExternalProvider>,
    )
    .or_else(
        "second-leased",
        Arc::new(LeasedMock { name: "beta" }) as Arc<dyn ExternalProvider>,
    );

    // Lease attributed to `beta` (the second child) — naïve
    // "first-leased-child wins" routing would send this to `alpha`,
    // which now defensively rejects it.
    let lease_beta = sample_lease("beta", "lease-from-beta");
    let renewed = chain
        .renew(&lease_beta)
        .await
        .expect("renew routed to beta");
    assert_eq!(
        renewed.secret.expose_secret(),
        "renewed:lease-from-beta@beta"
    );

    let revoked = chain
        .revoke(&lease_beta)
        .await
        .expect("revoke routed to beta");
    assert_eq!(
        revoked.secret.expose_secret(),
        "revoked:lease-from-beta@beta"
    );

    // Unknown attribution surfaces as NotFound (the chain does not
    // silently fall back to "any leased child").
    let lease_unknown = sample_lease("gamma", "lease-from-gamma");
    let err = chain
        .renew(&lease_unknown)
        .await
        .expect_err("unattributed lease must fail");
    assert!(matches!(err, ProviderError::NotFound { .. }));
}

#[tokio::test]
async fn renew_and_revoke_round_trip_through_arc_dyn() {
    // Exercise the trait shape through `Arc<dyn LeasedProvider>` so
    // dyn-safety is verified at compile time, and `renew` / `revoke`
    // wire end-to-end through the future envelope.
    let p: Arc<dyn LeasedProvider> = Arc::new(LeasedMock { name: "vault-stub" });
    let lease = sample_lease("vault-stub", "lease-abc");

    let renewed = p.renew(&lease).await.expect("renew ok");
    assert_eq!(
        renewed.secret.expose_secret(),
        "renewed:lease-abc@vault-stub"
    );

    let revoked = p.revoke(&lease).await.expect("revoke ok");
    assert_eq!(
        revoked.secret.expose_secret(),
        "revoked:lease-abc@vault-stub"
    );
}

#[test]
fn default_handles_lease_uses_provider_name_attribution() {
    let p = LeasedMock { name: "vault-stub" };
    assert!(
        p.handles_lease(&sample_lease("vault-stub", "any")),
        "matching provider_name → handles_lease true"
    );
    assert!(
        !p.handles_lease(&sample_lease("other", "any")),
        "mismatched attribution → handles_lease false"
    );
}

#[test]
fn provider_error_is_send_sync_for_leased_paths() {
    // Statically-asserted ergonomics: `ProviderError` (used by the
    // future returned from renew/revoke) crosses `.await` points, so
    // it must be `Send + Sync` to be useful through `Arc<dyn …>`.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ProviderError>();
}
