//! Verifies the typed-CredentialId refresh path goes through
//! `RefreshCoordinator::refresh_coalesced` (Stage 2.3 wiring).
//!
//! Mirrors `credential_thundering_herd_tests` but uses a parseable
//! `cred_<ULID>` id so the resolver routes through the two-tier
//! coalesce path (L1 + L2 in-memory claim) rather than the legacy
//! L1-only fallback.

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use nebula_credential::{
    Credential, CredentialContext, CredentialMetadata, CredentialStore, Refreshable, SecretString,
    error::CredentialError,
    resolve::{RefreshOutcome, RefreshPolicy, ResolveResult},
    scheme::SecretToken,
    store::{PutMode, StoredCredential},
};
use nebula_engine::credential::{
    CredentialResolver,
    refresh::{RefreshCoordConfig, RefreshCoordinator},
};
use nebula_schema::FieldValues;
use nebula_storage::credential::{
    InMemoryRefreshClaimRepo, InMemoryStore, RefreshClaimRepo, ReplicaId,
};

static REFRESH_COUNT: AtomicU32 = AtomicU32::new(0);

#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop,
)]
struct TwoTierState {
    token: String,
    #[zeroize(skip)]
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl nebula_credential::CredentialState for TwoTierState {
    const KIND: &'static str = "two_tier_test";
    const VERSION: u32 = 1;

    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        Some(self.expires_at)
    }
}

struct TwoTierCredential;

impl Credential for TwoTierCredential {
    type Input = FieldValues;
    type Scheme = SecretToken;
    type State = TwoTierState;

    const KEY: &'static str = "two_tier_test";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::new(
            nebula_core::credential_key!("two_tier_test"),
            "Two-tier test",
            "Test credential for two-tier coalesce wiring",
            Self::schema(),
            nebula_credential::AuthPattern::SecretToken,
        )
    }

    fn project(state: &TwoTierState) -> SecretToken {
        SecretToken::new(SecretString::new(state.token.clone()))
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<TwoTierState, ()>, CredentialError> {
        unreachable!("not used")
    }
}

impl Refreshable for TwoTierCredential {
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy {
        early_refresh: std::time::Duration::from_mins(5),
        jitter: std::time::Duration::ZERO,
        ..RefreshPolicy::DEFAULT
    };

    async fn refresh(
        state: &mut TwoTierState,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        REFRESH_COUNT.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        state.token = "refreshed-typed-token".to_owned();
        state.expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
        Ok(RefreshOutcome::Refreshed)
    }
}

#[tokio::test]
async fn typed_id_routes_through_refresh_coalesced() {
    REFRESH_COUNT.store(0, Ordering::SeqCst);

    let store = Arc::new(InMemoryStore::new());
    // Generate a real `cred_<ULID>` id so the resolver takes the typed
    // path through `refresh_coalesced` (Stage 2.3 migration).
    let typed_id = nebula_core::CredentialId::new();
    let credential_id = typed_id.to_string();

    let expires_at = chrono::Utc::now() + chrono::Duration::minutes(2);
    let state = TwoTierState {
        token: "old-token".into(),
        expires_at,
    };
    let data = serde_json::to_vec(&state).unwrap();

    let cred = StoredCredential {
        id: credential_id.clone(),
        credential_key: "two_tier_test".into(),
        data,
        state_kind: "two_tier_test".into(),
        state_version: 1,
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        expires_at: Some(expires_at),
        reauth_required: false,
        metadata: Default::default(),
    };
    store.put(cred, PutMode::CreateOnly).await.unwrap();

    // Build a coordinator wired with an in-memory `RefreshClaimRepo` so
    // the L2 path is observable end-to-end.
    let claim_repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    let coord = Arc::new(
        RefreshCoordinator::new_with(
            Arc::clone(&claim_repo),
            ReplicaId::new("test-A"),
            RefreshCoordConfig::default(),
        )
        .expect("default config valid"),
    );

    let resolver =
        Arc::new(CredentialResolver::new(store).with_refresh_coordinator(Arc::clone(&coord)));
    let ctx = CredentialContext::for_test("test-user");

    // Fire 10 concurrent resolves on the typed id; the two-tier
    // coalesce must collapse them to a single inner refresh.
    let mut handles = Vec::with_capacity(10);
    for _ in 0..10 {
        let r = Arc::clone(&resolver);
        let c = ctx.clone();
        let id = credential_id.clone();
        handles.push(tokio::spawn(async move {
            r.resolve_with_refresh::<TwoTierCredential>(&id, &c).await
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles).await;

    for (i, result) in results.iter().enumerate() {
        let inner = result.as_ref().expect("task should not panic");
        assert!(inner.is_ok(), "task {i} failed: {inner:?}");
    }

    for result in &results {
        let handle = result.as_ref().unwrap().as_ref().unwrap();
        let value = handle.snapshot().token().expose_secret().to_owned();
        assert_eq!(value, "refreshed-typed-token");
    }

    // Two-tier coalesce: exactly one refresh.
    assert_eq!(
        REFRESH_COUNT.load(Ordering::SeqCst),
        1,
        "two-tier coalesce must collapse 10 concurrent calls to 1 refresh"
    );

    // After release the L2 row should be deleted — a fresh acquire from
    // a different replica id wins immediately.
    let attempt = claim_repo
        .try_claim(
            &typed_id,
            &ReplicaId::new("test-B"),
            std::time::Duration::from_secs(5),
        )
        .await
        .expect("repo try_claim ok");
    assert!(
        matches!(
            attempt,
            nebula_storage::credential::ClaimAttempt::Acquired(_)
        ),
        "after refresh_coalesced release, row must be reclaimable: {attempt:?}"
    );
}
