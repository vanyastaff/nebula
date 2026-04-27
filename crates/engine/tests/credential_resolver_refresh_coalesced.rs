//! Verifies the typed-CredentialId refresh path goes through
//! `RefreshCoordinator::refresh_coalesced` (Stage 2.3 wiring).
//!
//! Mirrors `credential_thundering_herd_tests` but uses a parseable
//! `cred_<ULID>` id so the resolver routes through the two-tier
//! coalesce path (L1 + L2 in-memory claim) rather than the legacy
//! L1-only fallback.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicUsize, Ordering},
    },
    time::Duration,
};

use chrono::{DateTime, Utc};
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
    ClaimAttempt, ClaimToken, HeartbeatError, InMemoryRefreshClaimRepo, InMemoryStore,
    ReclaimedClaim, RefreshClaimRepo, ReplicaId, RepoError,
};

/// Counting wrapper around [`RefreshClaimRepo`] used to verify the L2
/// path is actually exercised by the two-tier coalesce. Forwards every
/// trait method to `inner`; the assertions in the test inspect the
/// `try_claim_calls` / `release_calls` counters directly.
struct CountingClaimRepo {
    inner: Arc<dyn RefreshClaimRepo>,
    try_claim_calls: AtomicUsize,
    release_calls: AtomicUsize,
}

#[async_trait::async_trait]
impl RefreshClaimRepo for CountingClaimRepo {
    async fn try_claim(
        &self,
        credential_id: &nebula_core::CredentialId,
        holder: &ReplicaId,
        ttl: Duration,
    ) -> Result<ClaimAttempt, RepoError> {
        self.try_claim_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.try_claim(credential_id, holder, ttl).await
    }

    async fn heartbeat(&self, token: &ClaimToken, ttl: Duration) -> Result<(), HeartbeatError> {
        self.inner.heartbeat(token, ttl).await
    }

    async fn release(&self, token: ClaimToken) -> Result<(), RepoError> {
        self.release_calls.fetch_add(1, Ordering::SeqCst);
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
        credential_id: &nebula_core::CredentialId,
        crashed_holder: &ReplicaId,
        generation: u64,
    ) -> Result<(), RepoError> {
        self.inner
            .record_sentinel_event(credential_id, crashed_holder, generation)
            .await
    }

    async fn count_sentinel_events_in_window(
        &self,
        credential_id: &nebula_core::CredentialId,
        window_start: DateTime<Utc>,
    ) -> Result<u32, RepoError> {
        self.inner
            .count_sentinel_events_in_window(credential_id, window_start)
            .await
    }
}

static REFRESH_COUNT: AtomicU32 = AtomicU32::new(0);

#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop,
)]
struct TwoTierState {
    token: String,
    #[zeroize(skip)]
    expires_at: DateTime<Utc>,
}

impl nebula_credential::CredentialState for TwoTierState {
    const KIND: &'static str = "two_tier_test";
    const VERSION: u32 = 1;

    fn expires_at(&self) -> Option<DateTime<Utc>> {
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
        early_refresh: Duration::from_mins(5),
        jitter: Duration::ZERO,
        ..RefreshPolicy::DEFAULT
    };

    async fn refresh(
        state: &mut TwoTierState,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        REFRESH_COUNT.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;
        state.token = "refreshed-typed-token".to_owned();
        state.expires_at = Utc::now() + chrono::Duration::hours(1);
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

    let expires_at = Utc::now() + chrono::Duration::minutes(2);
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
        created_at: Utc::now(),
        updated_at: Utc::now(),
        expires_at: Some(expires_at),
        reauth_required: false,
        metadata: Default::default(),
    };
    store.put(cred, PutMode::CreateOnly).await.unwrap();

    // Build a coordinator wired with an in-memory `RefreshClaimRepo`
    // wrapped in a counting double, so the test directly observes that
    // the L2 path was exercised (try_claim + release counts ≥ 1) — this
    // distinguishes a real two-tier coalesce from an L1-only fallback
    // that would silently pass `REFRESH_COUNT == 1` without ever
    // touching the L2 row.
    let inner_repo: Arc<dyn RefreshClaimRepo> = Arc::new(InMemoryRefreshClaimRepo::new());
    let counter = Arc::new(CountingClaimRepo {
        inner: Arc::clone(&inner_repo),
        try_claim_calls: AtomicUsize::new(0),
        release_calls: AtomicUsize::new(0),
    });
    let claim_repo: Arc<dyn RefreshClaimRepo> = Arc::clone(&counter) as _;
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

    // L2 path observation: REFRESH_COUNT == 1 alone is satisfied by an
    // L1-only fallback that never touches the L2 row. Distinguish a real
    // two-tier coalesce by asserting the wrapped `RefreshClaimRepo` saw
    // at least one `try_claim` and one `release` for this credential —
    // both are emitted only on the L2 path.
    assert!(
        counter.try_claim_calls.load(Ordering::SeqCst) >= 1,
        "L2 try_claim must have been invoked at least once"
    );
    assert!(
        counter.release_calls.load(Ordering::SeqCst) >= 1,
        "L2 release must have been invoked at least once"
    );

    // After release the L2 row should be deleted — a fresh acquire from
    // a different replica id wins immediately.
    let attempt = claim_repo
        .try_claim(&typed_id, &ReplicaId::new("test-B"), Duration::from_secs(5))
        .await
        .expect("repo try_claim ok");
    assert!(
        matches!(attempt, ClaimAttempt::Acquired(_)),
        "after refresh_coalesced release, row must be reclaimable: {attempt:?}"
    );
}
