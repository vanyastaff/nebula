//! Facade lifecycle E2E (Increment 3b): exercise `CredentialService` refresh /
//! revoke / binding-validation against a credential type the first-party set
//! lacks — one that is **non-interactive *and* Revocable *and* Refreshable**.
//!
//! No first-party builtin fits: `api_key`/`basic_auth` aren't Revocable, and
//! `oauth2` is interactive (can't be created without the OAuth handshake). So
//! the harness registers a local `TestLifecycleCred` via the custom-registry
//! factory variant ([`with_memory_store_parts`]) over the real
//! `Audit(Cache(Encryption(SQLite)))` stack.
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

use std::sync::Arc;
use std::time::Duration;

use nebula_api::ports::credential_service_factory::with_memory_store_parts;
use nebula_core::auth::{
    AuthPattern, AuthScheme, EgressShape, RefreshStrategyKind, SchemeFamily, SensitiveScheme,
};
use nebula_credential::error::CredentialError;
use nebula_credential::resolve::{RefreshOutcome, ResolveResult};
use nebula_credential::{
    CredentialContext, CredentialDisplay, CredentialMetadata, CredentialRegistry,
    CredentialService, CredentialServiceError, DispatchOps, DynCredentialStore, ErasedPendingStore,
    TenantScope, ValidatedCredentialBindingError, identity_state, register_refreshable_ops,
    register_revocable_ops, register_runtime_ops, schema_of,
};
use nebula_schema::{FieldValues, Schema};
use nebula_storage::credential::EnvKeyProvider;
use serde::{Deserialize, Serialize};
use serde_json::json;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// 32 `0x42` bytes, base64 — a valid AES-256 key fixture (mirrors the factory's
/// dev key). Not a secret: a fixed test constant.
const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

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
#[allow(dead_code)]
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
        Ok(ResolveResult::Complete(TestScheme {
            token: token.to_owned(),
            generation: 1,
        }))
    }

    async fn refresh(
        state: &mut TestScheme,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        // Simulate a provider-contacting rotation: bump the material so the
        // facade takes the `Rewrote` success arm (which must stamp
        // `last_validated_at`).
        state.generation += 1;
        state.token = format!("v{}", state.generation);
        Ok(RefreshOutcome::Refreshed)
    }

    async fn revoke(
        state: &mut TestScheme,
        _ctx: &CredentialContext,
    ) -> Result<(), CredentialError> {
        // Provider-side revoke succeeds; the facade writes the tombstone and
        // drops the bytes. Clearing here mirrors a real impl zeroing material.
        state.token.clear();
        Ok(())
    }
}

// ── Harness ────────────────────────────────────────────────────────────

async fn build_service() -> Arc<CredentialService> {
    let key = Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte AES key"));

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

    with_memory_store_parts(key, registry, ops)
        .await
        .expect("service composes (advertised caps match ops)")
}

/// Read the persisted `last_validated_at` straight off the layered store (the
/// service head does not expose it).
async fn last_validated(svc: &CredentialService, id: &str) -> chrono::DateTime<chrono::Utc> {
    let store: Arc<dyn DynCredentialStore> = svc.credential_store_handle();
    let row = store.get(id).await.expect("stored row present");
    row.last_validated_at()
        .expect("a created/refreshed credential carries a last_validated_at stamp")
}

fn scope() -> TenantScope {
    TenantScope::new("org", "ws")
}

async fn create_cred(svc: &CredentialService) -> String {
    svc.create(
        &scope(),
        "test_lifecycle",
        json!({ "token": "v1" }),
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
