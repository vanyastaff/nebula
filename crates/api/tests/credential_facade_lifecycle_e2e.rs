//! Facade lifecycle E2E (Increment 3b): exercise `CredentialService` refresh /
//! revoke / binding-validation against a credential type the default API set
//! lacks — one that is **non-interactive *and* Revocable *and* Refreshable**.
//!
//! The default static credentials (`api_key`, `basic_auth`, `signing_key`) are
//! not Revocable or Refreshable; interactive `oauth2` is parked until the
//! universal pending-flow transport is integrated. The harness therefore
//! registers a local `TestLifecycleCred` via the custom-registry factory
//! variant ([`with_memory_store_parts`]) over the real
//! `Audit(Encryption(SQLite))` stack.
//!
//! Regressions pinned here (the facade refresh/revoke paths have no in-crate
//! unit coverage — `nebula-credential` cannot host this harness without a
//! `credential→storage` dev-dep cycle):
//! - `refresh` advances `last_validated_at` (the FIX-2 wiring bug that shipped
//!   dead and was caught only by static review — no test had exercised it).
//! - a revoked credential is unresolvable: `refresh`/`get` fail closed as
//!   `NotFound` (no resurrection through the service).
//! - `validate_credential_binding` rejects a tombstoned id with the typed
//!   `CredentialTombstoned` (Q9 end-to-end).

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use nebula_api::ports::credential_service_factory::{
    with_memory_store_external, with_memory_store_parts,
};
use nebula_core::auth::{
    AuthPattern, AuthScheme, EgressShape, RefreshStrategyKind, SchemeFamily, SensitiveScheme,
};
use nebula_credential::error::CredentialError;
use nebula_credential::provider::{
    ExternalProvider, ExternalReference, ProviderError, ProviderFuture,
};
use nebula_credential::resolve::{RefreshOutcome, ResolveResult};
use nebula_credential::{
    CredentialContext, CredentialDisplay, CredentialMetadata, CredentialRegistry,
    CredentialService, CredentialServiceError, DispatchOps, ErasedPendingStore, TenantScope,
    ValidatedCredentialBindingError, identity_state, register_refreshable_ops,
    register_revocable_ops, register_runtime_ops, schema_of,
};
use nebula_schema::{FieldValues, Schema};
use nebula_storage::credential::EnvKeyProvider;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Notify;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// 32 `0x42` bytes, base64 — a valid AES-256 key fixture (mirrors the factory's
/// dev key). Not a secret: a fixed test constant.
const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";
static STALE_UPDATE_RESOLVE_CALLS: AtomicUsize = AtomicUsize::new(0);
static COALESCED_REFRESH_CALLS: AtomicUsize = AtomicUsize::new(0);
static DROPPED_REFRESH_CALLS: AtomicUsize = AtomicUsize::new(0);
static COALESCED_REVOKE_CALLS: AtomicUsize = AtomicUsize::new(0);
static DROPPED_REVOKE_CALLS: AtomicUsize = AtomicUsize::new(0);
static COALESCED_REFRESH_ENTERED: OnceLock<Notify> = OnceLock::new();
static COALESCED_REFRESH_CONTINUE: OnceLock<Notify> = OnceLock::new();
static DROPPED_REFRESH_ENTERED: OnceLock<Notify> = OnceLock::new();
static DROPPED_REFRESH_CONTINUE: OnceLock<Notify> = OnceLock::new();
static COALESCED_REVOKE_ENTERED: OnceLock<Notify> = OnceLock::new();
static COALESCED_REVOKE_CONTINUE: OnceLock<Notify> = OnceLock::new();
static DROPPED_REVOKE_ENTERED: OnceLock<Notify> = OnceLock::new();
static DROPPED_REVOKE_CONTINUE: OnceLock<Notify> = OnceLock::new();

fn signal(cell: &'static OnceLock<Notify>) -> &'static Notify {
    cell.get_or_init(Notify::new)
}

// ── A non-interactive, Refreshable, Revocable test credential ──────────

/// Active family declaring an engine-drivable `RefreshToken` class so a
/// `Refreshable` credential passes the F3 containment law at registration
/// (`SchemeFamily::supports_active_refresh`).
struct TestRefreshFamily;

impl SchemeFamily for TestRefreshFamily {
    const EGRESS: &'static [EgressShape] = &[EgressShape::InlineSecret];
    fn refresh_classes() -> &'static [RefreshStrategyKind] {
        &[RefreshStrategyKind::RefreshToken]
    }
    fn pattern() -> AuthPattern {
        AuthPattern::OAuth2
    }
}

/// Stored state == projected scheme (identity). Holds the secret bytes so it is
/// `Sensitive` (zeroized on drop); `generation` lets `refresh` produce visibly
/// rotated material.
#[derive(Serialize, Deserialize, Clone, Zeroize, ZeroizeOnDrop)]
struct TestScheme {
    token: String,
    generation: u32,
}

impl AuthScheme for TestScheme {
    type Family = TestRefreshFamily;
    fn pattern() -> AuthPattern {
        AuthPattern::OAuth2
    }
}

impl SensitiveScheme for TestScheme {}

identity_state!(TestScheme, "test_lifecycle_state", 1);

/// Create-form properties.
//
// `token` is read by the `#[derive(Schema)]`/`Deserialize` derives and consumed
// at runtime through `FieldValues` in `resolve`, never via direct field access —
// `dead_code` cannot see those paths on a private test struct.
#[derive(Schema, Deserialize, Default)]
#[expect(dead_code)]
struct TestProps {
    /// Initial secret token.
    #[field(secret, label = "Token")]
    #[validate(required)]
    token: String,
}

/// The credential type under test. `#[credential]` reads the methods present —
/// here `refresh` + `revoke` (and no interactive/test method) — and emits the
/// `Credential`/`Refreshable`/`Revocable` impls + the capability-report consts
/// (refreshable + revocable true; interactive/testable/dynamic false).
struct TestLifecycleCred;

#[nebula_credential::credential(key = "test_lifecycle")]
impl TestLifecycleCred {
    type Properties = TestProps;
    type Scheme = TestScheme;
    type State = TestScheme;

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("test_lifecycle"))
            .name("Test Lifecycle Credential")
            .description("non-interactive refreshable+revocable credential for facade E2E tests")
            .schema(schema_of::<Self::Properties>())
            .pattern(AuthPattern::OAuth2)
            .build()
            .expect("test_lifecycle metadata is valid")
    }

    fn project(state: &TestScheme) -> TestScheme {
        state.clone()
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<TestScheme, ()>, CredentialError> {
        let token = values.get_string_by_str("token").ok_or_else(|| {
            CredentialError::InvalidInput("missing required field 'token'".into())
        })?;
        if token == "must-not-resolve" {
            STALE_UPDATE_RESOLVE_CALLS.fetch_add(1, Ordering::SeqCst);
        }
        Ok(ResolveResult::Complete(TestScheme {
            token: token.to_owned(),
            generation: 1,
        }))
    }

    async fn refresh(
        state: &mut TestScheme,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        match state.token.as_str() {
            "refresh-coalesce" => {
                COALESCED_REFRESH_CALLS.fetch_add(1, Ordering::SeqCst);
                signal(&COALESCED_REFRESH_ENTERED).notify_one();
                signal(&COALESCED_REFRESH_CONTINUE).notified().await;
            },
            "refresh-drop" => {
                DROPPED_REFRESH_CALLS.fetch_add(1, Ordering::SeqCst);
                signal(&DROPPED_REFRESH_ENTERED).notify_one();
                signal(&DROPPED_REFRESH_CONTINUE).notified().await;
            },
            _ => {},
        }
        // Simulate a provider-contacting rotation: bump the material so the
        // facade takes the `Rewrote` success arm (which must stamp
        // `last_validated_at`).
        state.generation += 1;
        if !matches!(state.token.as_str(), "refresh-coalesce" | "refresh-drop") {
            state.token = format!("v{}", state.generation);
        }
        Ok(RefreshOutcome::Refreshed)
    }

    async fn revoke(
        state: &mut TestScheme,
        _ctx: &CredentialContext,
    ) -> Result<(), CredentialError> {
        match state.token.as_str() {
            "revoke-coalesce" => {
                COALESCED_REVOKE_CALLS.fetch_add(1, Ordering::SeqCst);
                signal(&COALESCED_REVOKE_ENTERED).notify_one();
                signal(&COALESCED_REVOKE_CONTINUE).notified().await;
            },
            "revoke-drop" => {
                DROPPED_REVOKE_CALLS.fetch_add(1, Ordering::SeqCst);
                signal(&DROPPED_REVOKE_ENTERED).notify_one();
                signal(&DROPPED_REVOKE_CONTINUE).notified().await;
            },
            _ => {},
        }
        // Provider-side revoke succeeds; the facade writes the tombstone and
        // drops the bytes. Clearing here mirrors a real impl zeroing material.
        state.token.clear();
        Ok(())
    }
}

// ── Harness ────────────────────────────────────────────────────────────

fn test_key() -> Arc<EnvKeyProvider> {
    Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte AES key"))
}

fn test_registry_and_ops() -> (CredentialRegistry, DispatchOps<ErasedPendingStore>) {
    let mut registry = CredentialRegistry::new();
    registry
        .register(TestLifecycleCred, "nebula-api-test")
        .expect("test_lifecycle registers (unique key)");

    let mut ops = DispatchOps::<ErasedPendingStore>::new();
    register_runtime_ops::<TestLifecycleCred, ErasedPendingStore>(&mut ops).expect("runtime ops");
    register_refreshable_ops::<TestLifecycleCred, ErasedPendingStore>(&mut ops)
        .expect("refreshable ops");
    register_revocable_ops::<TestLifecycleCred, ErasedPendingStore>(&mut ops)
        .expect("revocable ops");
    (registry, ops)
}

async fn build_service() -> Arc<CredentialService> {
    let (registry, ops) = test_registry_and_ops();
    with_memory_store_parts(test_key(), registry, ops)
        .await
        .expect("service composes (advertised caps match ops)")
}

/// A service whose state source is an external provider with no resolution
/// bridge wired — every resolution path must fail closed.
async fn build_external_service() -> Arc<CredentialService> {
    let (registry, ops) = test_registry_and_ops();
    with_memory_store_external(test_key(), registry, ops, Arc::new(StubExternalProvider))
        .await
        .expect("service composes over an external (unwired) source")
}

/// Stub external provider: its `resolve` is never reached because the source
/// gate fails closed first (the ADR-0051 resolution bridge is unbuilt).
#[derive(Debug)]
struct StubExternalProvider;

impl ExternalProvider for StubExternalProvider {
    fn resolve<'a>(&'a self, _reference: &'a ExternalReference) -> ProviderFuture<'a> {
        ProviderFuture::ready(Err(ProviderError::Unavailable {
            reason: "stub external provider — resolution bridge not wired".to_owned(),
        }))
    }

    // guard-justified: the `ExternalProvider` trait fixes the `-> &str` return,
    // so the lint's suggested `-> &'static str` cannot apply to this impl.
    #[expect(clippy::unnecessary_literal_bound)]
    fn provider_name(&self) -> &str {
        "stub-vault"
    }
}

/// Read the persisted validation anchor through the facade's secret-free head.
async fn last_validated(svc: &CredentialService, id: &str) -> chrono::DateTime<chrono::Utc> {
    svc.get(&scope(), id)
        .await
        .expect("stored row present")
        .last_validated_at
        .expect("a created/refreshed credential carries a last_validated_at stamp")
}

fn scope() -> TenantScope {
    TenantScope::new("org", "ws")
}

async fn create_cred(svc: &CredentialService) -> String {
    create_cred_with_token(svc, "v1").await
}

async fn create_cred_with_token(svc: &CredentialService, token: &str) -> String {
    svc.create(
        &scope(),
        "test_lifecycle",
        json!({ "token": token }),
        CredentialDisplay::default(),
    )
    .await
    .expect("create succeeds")
    .id
}

// ── Regressions ──────────────────────────────────────────────────────────

#[tokio::test]
async fn refresh_advances_last_validated_at() {
    // FIX-2 E2E: a provider-contacting facade refresh MUST advance the
    // re-validation anchor. The original FIX-2 built the stamped metadata but
    // never wired it into the written row, so the anchor stayed at creation
    // time — invisible to clippy and unit tests, caught only by static review.
    let svc = build_service().await;
    let id = create_cred(&svc).await;

    let t0 = last_validated(&svc, &id).await;
    // Guarantee a measurable gap so a discarded stamp (t1 == t0) is
    // distinguishable from an advanced one (t1 > t0).
    tokio::time::sleep(Duration::from_millis(10)).await;

    svc.refresh(&scope(), &id).await.expect("refresh succeeds");

    let t1 = last_validated(&svc, &id).await;
    assert!(
        t1 > t0,
        "refresh must advance last_validated_at (was {t0}, now {t1}) — a discarded \
         stamp would leave it at creation time"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn concurrent_management_refreshes_coalesce_to_one_provider_call() {
    COALESCED_REFRESH_CALLS.store(0, Ordering::SeqCst);
    let svc = build_service().await;
    let id = create_cred_with_token(&svc, "refresh-coalesce").await;

    let first = tokio::spawn({
        let svc = Arc::clone(&svc);
        let id = id.clone();
        async move { svc.refresh(&scope(), &id).await }
    });
    tokio::time::timeout(
        Duration::from_secs(1),
        signal(&COALESCED_REFRESH_ENTERED).notified(),
    )
    .await
    .expect("first provider refresh must enter");

    let second = tokio::spawn({
        let svc = Arc::clone(&svc);
        let id = id.clone();
        async move { svc.refresh(&scope(), &id).await }
    });
    // Give the second call time to complete its real SQLite load and park
    // behind the first request's L1 lease before releasing provider work.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        COALESCED_REFRESH_CALLS.load(Ordering::SeqCst),
        1,
        "the local waiter must not enter provider code"
    );

    signal(&COALESCED_REFRESH_CONTINUE).notify_one();
    let first_report = tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .expect("first refresh must finish")
        .expect("first task must not panic")
        .expect("first refresh must succeed");
    let second_report = tokio::time::timeout(Duration::from_secs(2), second)
        .await
        .expect("coalesced refresh must finish")
        .expect("second task must not panic")
        .expect("coalesced refresh must succeed");

    assert!(first_report.refreshed);
    assert!(second_report.refreshed);
    assert_eq!(first_report.head.version, 2);
    assert_eq!(second_report.head.version, 2);
    assert_eq!(COALESCED_REFRESH_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_management_refresh_caller_cannot_cancel_provider_and_cas() {
    DROPPED_REFRESH_CALLS.store(0, Ordering::SeqCst);
    let svc = build_service().await;
    let id = create_cred_with_token(&svc, "refresh-drop").await;

    let caller = tokio::spawn({
        let svc = Arc::clone(&svc);
        let id = id.clone();
        async move { svc.refresh(&scope(), &id).await }
    });
    tokio::time::timeout(
        Duration::from_secs(1),
        signal(&DROPPED_REFRESH_ENTERED).notified(),
    )
    .await
    .expect("provider refresh must enter before caller drop");

    caller.abort();
    let _ = caller.await;
    signal(&DROPPED_REFRESH_CONTINUE).notify_one();

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if svc
                .get(&scope(), &id)
                .await
                .is_ok_and(|head| head.version == 2)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owned provider+persistence task must commit after caller Drop");
    assert_eq!(DROPPED_REFRESH_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn concurrent_management_revokes_coalesce_to_one_provider_call() {
    COALESCED_REVOKE_CALLS.store(0, Ordering::SeqCst);
    let svc = build_service().await;
    let id = create_cred_with_token(&svc, "revoke-coalesce").await;

    let first = tokio::spawn({
        let svc = Arc::clone(&svc);
        let id = id.clone();
        async move { svc.revoke(&scope(), &id).await }
    });
    tokio::time::timeout(
        Duration::from_secs(1),
        signal(&COALESCED_REVOKE_ENTERED).notified(),
    )
    .await
    .expect("first provider revoke must enter");

    let second = tokio::spawn({
        let svc = Arc::clone(&svc);
        let id = id.clone();
        async move { svc.revoke(&scope(), &id).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(COALESCED_REVOKE_CALLS.load(Ordering::SeqCst), 1);

    signal(&COALESCED_REVOKE_CONTINUE).notify_one();
    tokio::time::timeout(Duration::from_secs(2), first)
        .await
        .expect("first revoke must finish")
        .expect("first task must not panic")
        .expect("first revoke must succeed");
    tokio::time::timeout(Duration::from_secs(2), second)
        .await
        .expect("coalesced revoke must finish")
        .expect("second task must not panic")
        .expect("coalesced revoke must be idempotent");
    assert_eq!(COALESCED_REVOKE_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_management_revoke_caller_cannot_cancel_provider_and_tombstone() {
    DROPPED_REVOKE_CALLS.store(0, Ordering::SeqCst);
    let svc = build_service().await;
    let id = create_cred_with_token(&svc, "revoke-drop").await;

    let caller = tokio::spawn({
        let svc = Arc::clone(&svc);
        let id = id.clone();
        async move { svc.revoke(&scope(), &id).await }
    });
    tokio::time::timeout(
        Duration::from_secs(1),
        signal(&DROPPED_REVOKE_ENTERED).notified(),
    )
    .await
    .expect("provider revoke must enter before caller drop");

    caller.abort();
    let _ = caller.await;
    signal(&DROPPED_REVOKE_CONTINUE).notify_one();

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if matches!(
                svc.get(&scope(), &id).await,
                Err(CredentialServiceError::NotFound { .. })
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owned provider+tombstone task must finish after caller Drop");
    assert_eq!(DROPPED_REVOKE_CALLS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stale_update_is_rejected_before_provider_resolution() {
    let svc = build_service().await;
    let id = create_cred(&svc).await;

    let error = svc
        .update(
            &scope(),
            &id,
            Some(json!({ "token": "must-not-resolve" })),
            Some(2),
            CredentialDisplay::default(),
        )
        .await
        .expect_err("a deterministically stale update must fail before provider work");

    assert!(matches!(
        error,
        CredentialServiceError::VersionConflict {
            expected: 2,
            actual: 1,
            ..
        }
    ));
    assert_eq!(
        STALE_UPDATE_RESOLVE_CALLS.load(Ordering::SeqCst),
        0,
        "stale optimistic concurrency is a semantic precondition, not only a storage CAS"
    );
}

#[tokio::test]
async fn revoke_then_refresh_and_get_are_not_found() {
    // A revoked credential is gone: neither a refresh nor a read may resurrect
    // or serve it. `load_owned` maps the tombstone to `NotFound`.
    let svc = build_service().await;
    let id = create_cred(&svc).await;

    svc.revoke(&scope(), &id).await.expect("revoke succeeds");

    let refreshed = svc.refresh(&scope(), &id).await;
    assert!(
        matches!(refreshed, Err(CredentialServiceError::NotFound { .. })),
        "refresh of a revoked credential must fail closed as NotFound, got {refreshed:?}"
    );

    let got = svc.get(&scope(), &id).await;
    assert!(
        matches!(got, Err(CredentialServiceError::NotFound { .. })),
        "a revoked credential must read as NotFound (idempotent revoke view), got {got:?}"
    );
}

#[tokio::test]
async fn validate_credential_binding_rejects_tombstoned() {
    // Q9 E2E: a slot binding against a revoked credential surfaces the typed
    // `CredentialTombstoned`, not a bare NotFound — so the workflow author
    // learns the slot stopped resolving because the credential was revoked.
    let svc = build_service().await;
    let id = create_cred(&svc).await;

    svc.validate_credential_binding(&scope(), &id)
        .await
        .expect("a live credential binds");

    svc.revoke(&scope(), &id).await.expect("revoke succeeds");

    let err = svc
        .validate_credential_binding(&scope(), &id)
        .await
        .expect_err("a tombstoned credential must not bind");
    assert!(
        matches!(
            err,
            ValidatedCredentialBindingError::CredentialTombstoned { .. }
        ),
        "expected CredentialTombstoned, got {err:?}"
    );
}

#[tokio::test]
async fn external_source_rejects_create() {
    // Wrong-source guard: the external resolution bridge (ADR-0051) is not
    // wired, so a service built with an external source must reject secret
    // resolution rather than fall back to reading local bytes. `create`
    // resolves props → state, so it fails closed with ExternalSourceNotWired.
    let svc = build_external_service().await;

    let err = svc
        .create(
            &scope(),
            "test_lifecycle",
            json!({ "token": "v1" }),
            CredentialDisplay::default(),
        )
        .await
        .expect_err("create against an unwired external source must fail closed");
    // Assert the provider value, not just the variant: a provider-mapping
    // regression (wrong/empty name) must fail this test, locking the
    // source → error contract.
    match err {
        CredentialServiceError::ExternalSourceNotWired { provider } => {
            assert_eq!(
                provider, "stub-vault",
                "the error must carry the configured provider's name"
            );
        },
        other => panic!("expected ExternalSourceNotWired, got {other:?}"),
    }
}
