//! Probe: fallback-on-interrupt for `CredentialService::refresh`.
//!
//! A transient provider failure (e.g. network blip) on refresh must
//! return the cached non-expired snapshot rather than propagating the
//! error. Terminal failures (token expired / revoked / auth) always
//! propagate regardless of cached state. Mirrors the
//! `aws-credential-types` `fallback_on_interrupt` pattern.

use nebula_credential_runtime::test_fixtures::{
    RefreshFailureScript, RefreshableFixtureCredential, set_refresh_failure,
};
use nebula_credential_runtime::test_support::in_memory_service_with_fixtures;
use nebula_credential_runtime::{CredentialServiceError, TenantScope};
use serde_json::json;

/// Helper — `in_memory_service_with_fixtures` returns `(service, refresh_counter)`.
async fn build() -> (
    nebula_credential_runtime::CredentialService,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    in_memory_service_with_fixtures().await
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_transient_falls_back_to_cached_when_non_expired() {
    let (svc, _refreshes) = build().await;
    let scope = TenantScope::new("org", "ws");

    // Seed: create a refreshable credential with the fixture's default
    // synthetic expiry of `now + 1h` (non-expired).
    svc.create(
        &scope,
        RefreshableFixtureCredential::KEY,
        json!({ "token": "seed-token" }),
        nebula_credential::CredentialDisplay::default(),
    )
    .await
    .expect("create seed");

    let heads = svc.list(&scope).await.expect("list");
    assert_eq!(heads.len(), 1);
    let id = &heads[0].id;

    // Cached head reflects the seeded row.
    let before = svc.get(&scope, id).await.expect("pre-refresh get");

    // Script a transient failure for the next refresh call.
    set_refresh_failure(Some(RefreshFailureScript::Transient));

    // refresh() must NOT propagate the transient — it must return the
    // cached head because the stored material is still non-expired, and
    // the report must say so honestly.
    let report = svc
        .refresh(&scope, id)
        .await
        .expect("refresh falls back to cached non-expired head");
    assert!(
        !report.refreshed,
        "the fallback path must report refreshed = false"
    );

    // The returned head is the cached one (same key, same store version —
    // no write happened).
    assert_eq!(report.head.credential_key, before.credential_key);
    assert_eq!(report.head.version, before.version);

    // Clean any leftover script (defensive — `.take()` already consumed it).
    set_refresh_failure(None);
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_terminal_failure_propagates() {
    let (svc, _refreshes) = build().await;
    let scope = TenantScope::new("org", "ws");

    svc.create(
        &scope,
        RefreshableFixtureCredential::KEY,
        json!({ "token": "seed-token" }),
        nebula_credential::CredentialDisplay::default(),
    )
    .await
    .expect("create seed");

    let heads = svc.list(&scope).await.expect("list");
    let id = &heads[0].id;

    // Script a TERMINAL failure (TokenExpired) for the next refresh call.
    set_refresh_failure(Some(RefreshFailureScript::Terminal));

    let err = svc
        .refresh(&scope, id)
        .await
        .expect_err("terminal failure must propagate");

    assert!(
        matches!(err, CredentialServiceError::Provider(_)),
        "expected terminal Provider error, got {err:?}"
    );

    set_refresh_failure(None);
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_no_failure_returns_refreshed_snapshot() {
    // Sanity arm: with no scripted failure, refresh succeeds and bumps
    // the version (the fixture rotates the token + increments
    // `refresh_count` deterministically).
    let (svc, _refreshes) = build().await;
    let scope = TenantScope::new("org", "ws");

    svc.create(
        &scope,
        RefreshableFixtureCredential::KEY,
        json!({ "token": "seed-token" }),
        nebula_credential::CredentialDisplay::default(),
    )
    .await
    .expect("create seed");

    let heads = svc.list(&scope).await.expect("list");
    let id = &heads[0].id;

    let before = svc.get(&scope, id).await.expect("get");

    let report = svc.refresh(&scope, id).await.expect("refresh ok");
    assert!(
        report.refreshed,
        "a real refresh must report refreshed = true"
    );

    assert_eq!(report.head.credential_key, before.credential_key);
    // A successful refresh re-persists the row via CAS in `refresh_inner`,
    // bumping the store version. The CAS version is the resolution-independent
    // proof a write happened — assert it strictly increased. `updated_at` is
    // only millisecond-resolution on the durable backend, so two writes that
    // land in the same millisecond share a timestamp; it must never move
    // backwards, but is not a reliable strict-monotonic probe.
    assert!(
        report.head.version > before.version,
        "successful refresh must bump the store version (before={}, after={})",
        before.version,
        report.head.version,
    );
    assert!(
        report.head.updated_at >= before.updated_at,
        "updated_at must not move backwards (before={:?}, after={:?})",
        before.updated_at,
        report.head.updated_at,
    );
}
