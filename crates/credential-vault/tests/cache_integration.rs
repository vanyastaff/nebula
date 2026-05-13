//! Integration tests proving the Phase A `ProviderCacheLayer` composes
//! correctly over the Phase C `VaultProvider` backend.
//!
//! These run with the same wiremock pattern as the unit tests but cross
//! the crate boundary into `nebula-storage` so we exercise the real
//! cache layer rather than a hand-rolled in-test cache.

use std::{sync::Arc, time::Duration};

use nebula_credential::{
    SecretString,
    provider::{ExternalProvider, ExternalReference, ProviderKind},
};
use nebula_credential_vault::{VaultConfig, VaultProvider};
use nebula_storage::credential::{ProviderCacheConfig, ProviderCacheLayer};
use serde_json::json;
use url::Url;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn vault_at(server: &MockServer) -> Arc<VaultProvider> {
    let url = Url::parse(&server.uri()).expect("mock server URL parses");
    Arc::new(
        VaultProvider::new(VaultConfig::new(
            url,
            SecretString::new("integration-token"),
        ))
        .expect("provider builds"),
    )
}

fn kv_ref(p: &str) -> ExternalReference {
    ExternalReference {
        provider: ProviderKind::Vault,
        path: p.to_owned(),
        version: None,
        field: Some("value".to_owned()),
    }
}

#[tokio::test]
async fn kv_v2_resolution_is_cached_under_default_ttl() {
    // `ProviderCacheConfig::default()` sets `default_ttl: ZERO` — KV v2
    // responses carry no provider-supplied TTL, so the effective TTL is
    // ZERO (bypass). Configure a positive `default_ttl` so the layer
    // actually caches the static response, then prove the second resolve
    // does not reach Vault.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/secret/data/cached"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "data": { "value": "v1" }, "metadata": { "version": 1 } }
        })))
        .expect(1) // exactly one upstream call across both resolves
        .mount(&server)
        .await;

    let inner = vault_at(&server);
    let layer = ProviderCacheLayer::new(
        inner as Arc<dyn ExternalProvider>,
        ProviderCacheConfig {
            max_entries: 100,
            default_ttl: Duration::from_mins(1),
        },
    );

    let first = layer
        .resolve(&kv_ref("cached"))
        .await
        .expect("first resolve");
    let second = layer
        .resolve(&kv_ref("cached"))
        .await
        .expect("second resolve");
    assert_eq!(first.secret.expose_secret(), "v1");
    assert_eq!(second.secret.expose_secret(), "v1");
    // Mock's `.expect(1)` is asserted on drop, but a hit-rate sanity check
    // here surfaces the failure with a clearer message in the test output.
    let stats = layer.stats();
    assert_eq!(stats.misses, 1, "exactly one inner call recorded");
    assert!(stats.hits >= 1, "second resolve must have been a cache hit");
}

#[tokio::test]
async fn dynamic_resolution_is_cached_using_response_ttl() {
    // Dynamic resolutions carry a `lease.ttl` ⇒ resolution.ttl = lease.ttl,
    // so the cache layer caches even with the default ZERO fallback — the
    // per-entry TTL comes from the provider response. This is the canonical
    // use case for the cache layer (ADR-0051 §3.2 of the cache plan).
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/database/creds/role-a"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "lease_id": "database/creds/role-a/abc",
            "lease_duration": 600,
            "renewable": true,
            "data": { "value": "p-1" }
        })))
        .expect(1) // one upstream call → second resolve served from cache
        .mount(&server)
        .await;

    let inner = vault_at(&server);
    let layer = ProviderCacheLayer::new(
        inner as Arc<dyn ExternalProvider>,
        ProviderCacheConfig::default(),
    );

    let reference = kv_ref("dyn/database/creds/role-a");
    let first = layer.resolve(&reference).await.expect("first resolve");
    let second = layer.resolve(&reference).await.expect("second resolve");
    assert_eq!(first.secret.expose_secret(), "p-1");
    assert_eq!(second.secret.expose_secret(), "p-1");

    // The cached resolution must keep the lease the inner provider
    // attributed to itself — the cache layer is a pass-through for that
    // metadata, otherwise renew/revoke routing would lose attribution.
    let cached_lease = second
        .lease
        .as_ref()
        .expect("dynamic resolution carries a lease");
    assert_eq!(cached_lease.provider, "vault");
    assert_eq!(cached_lease.lease_id, "database/creds/role-a/abc");
}

#[tokio::test]
async fn revoke_invalidates_cache_and_calls_vault_once() {
    // The single load-bearing integration assertion: revoking through the
    // cache layer's `LeasedProvider` view both (a) drops the cached entry
    // so the next resolve must re-hit Vault, and (b) forwards exactly one
    // call to Vault's `/sys/leases/revoke`. Without (a), a revoked lease
    // would remain visible from the cache until natural TTL expiry;
    // without (b), the lease would never reach the upstream revocation
    // path on the server.
    let server = MockServer::start().await;

    // Resolve mock: served twice — once before revoke (warms cache) and
    // once after revoke (proves invalidation, since post-revoke the cache
    // must miss). Vault would normally rotate credentials on the second
    // call; we use distinct lease ids so we can also verify the post-revoke
    // resolve picked up a fresh resolution.
    let resolve_responder = ResponseTemplate::new(200).set_body_json(json!({
        "lease_id": "database/creds/role-b/first",
        "lease_duration": 300,
        "renewable": true,
        "data": { "value": "before" }
    }));
    Mock::given(method("GET"))
        .and(path("/v1/database/creds/role-b"))
        .respond_with(resolve_responder)
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/database/creds/role-b"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "lease_id": "database/creds/role-b/second",
            "lease_duration": 300,
            "renewable": true,
            "data": { "value": "after" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    // Revoke mock: must be hit exactly once — this is the integration
    // contract Phase A wired in (cache.revoke ⇒ inner.revoke).
    Mock::given(method("PUT"))
        .and(path("/v1/sys/leases/revoke"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let inner = vault_at(&server);
    let layer = ProviderCacheLayer::new(
        inner as Arc<dyn ExternalProvider>,
        ProviderCacheConfig::default(),
    );

    let reference = kv_ref("dyn/database/creds/role-b");

    // First resolve: warms the cache with lease "first".
    let first = layer.resolve(&reference).await.expect("first resolve");
    assert_eq!(first.secret.expose_secret(), "before");
    let lease = first.lease.as_ref().expect("dynamic lease").clone();

    // Second resolve hits the cache — should NOT reach the mock.
    let cached = layer.resolve(&reference).await.expect("cached resolve");
    assert_eq!(cached.secret.expose_secret(), "before");

    // Revoke through the cache-layer view: must invalidate the cached
    // entry AND forward to Vault.
    let view = layer
        .lease_renewal()
        .expect("cache layer surfaces lease capability");
    let revoke_result = view.revoke(&lease).await.expect("revoke succeeds");
    assert!(
        revoke_result.secret.expose_secret().is_empty(),
        "revoke success returns the empty marker"
    );

    // Third resolve: cache MUST miss (the entry was invalidated) and Vault
    // returns the second lease.
    let after = layer
        .resolve(&reference)
        .await
        .expect("post-revoke resolve");
    assert_eq!(after.secret.expose_secret(), "after");
    let after_lease = after.lease.as_ref().expect("post-revoke lease");
    assert_eq!(
        after_lease.lease_id, "database/creds/role-b/second",
        "post-revoke resolve must produce a fresh lease"
    );

    // All `.expect(N)` mock assertions fire on `server` drop; the
    // assertions above are a faster signal for the load-bearing properties.
}
