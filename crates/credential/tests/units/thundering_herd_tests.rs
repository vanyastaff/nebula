//! Integration test: thundering herd prevention on credential refresh.
//!
//! Validates that spawning 10 concurrent `resolve_with_refresh` calls
//! on an expiring credential results in exactly 1 actual refresh call.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use nebula_credential::SecretString;
use nebula_credential::context::CredentialContext;
use nebula_credential::credential::Credential;
use nebula_credential::description::CredentialDescription;
use nebula_credential::error::CredentialError;
use nebula_credential::pending::NoPendingState;
use nebula_credential::resolve::{RefreshOutcome, RefreshPolicy, StaticResolveResult};
use nebula_credential::scheme::BearerToken;
use nebula_credential::store::{PutMode, StoredCredential};
use nebula_credential::{CredentialResolver, CredentialStore, InMemoryStore};
use nebula_parameter::ParameterCollection;
use nebula_parameter::values::ParameterValues;

/// Global counter tracking how many times `refresh()` is actually called.
static REFRESH_COUNT: AtomicU32 = AtomicU32::new(0);

// ── Test credential state ────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ThunderingHerdState {
    token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl nebula_credential::state::CredentialState for ThunderingHerdState {
    const KIND: &'static str = "thundering_herd_test";
    const VERSION: u32 = 1;

    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        Some(self.expires_at)
    }
}

// ── Test credential type ─────────────────────────────────────────────

struct ThunderingHerdCredential;

impl Credential for ThunderingHerdCredential {
    type Scheme = BearerToken;
    type State = ThunderingHerdState;
    type Pending = NoPendingState;

    const KEY: &'static str = "thundering_herd_test";
    const REFRESHABLE: bool = true;
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy {
        early_refresh: std::time::Duration::from_secs(300),
        jitter: std::time::Duration::ZERO, // no jitter for deterministic test
        ..RefreshPolicy::DEFAULT
    };

    fn description() -> CredentialDescription {
        CredentialDescription {
            key: Self::KEY.to_owned(),
            name: "Thundering Herd Test".to_owned(),
            description: "Test credential for thundering herd prevention".to_owned(),
            icon: None,
            icon_url: None,
            documentation_url: None,
            properties: Self::parameters(),
        }
    }

    fn parameters() -> ParameterCollection {
        ParameterCollection::new()
    }

    fn project(state: &ThunderingHerdState) -> BearerToken {
        BearerToken::new(SecretString::new(state.token.clone()))
    }

    async fn resolve(
        _values: &ParameterValues,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<ThunderingHerdState>, CredentialError> {
        unreachable!("not used in thundering herd tests")
    }

    async fn refresh(
        state: &mut ThunderingHerdState,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        REFRESH_COUNT.fetch_add(1, Ordering::SeqCst);
        // Small delay to simulate a network call so waiters actually queue up
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        state.token = "refreshed-token".to_owned();
        state.expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
        Ok(RefreshOutcome::Refreshed)
    }
}

// ── Test ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn only_one_refresh_under_concurrent_access() {
    REFRESH_COUNT.store(0, Ordering::SeqCst);

    let store = Arc::new(InMemoryStore::new());

    // Token expires in 2 minutes — inside the 5-minute early_refresh window.
    let expires_at = chrono::Utc::now() + chrono::Duration::minutes(2);
    let state = ThunderingHerdState {
        token: "old-token".into(),
        expires_at,
    };
    let data = serde_json::to_vec(&state).unwrap();

    let cred = StoredCredential {
        id: "herd-cred".into(),
        credential_key: "thundering_herd_test".into(),
        data,
        state_kind: "thundering_herd_test".into(),
        state_version: 1,
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        expires_at: Some(expires_at),
        metadata: Default::default(),
    };
    store.put(cred, PutMode::CreateOnly).await.unwrap();

    let resolver = Arc::new(CredentialResolver::new(store));
    let ctx = CredentialContext::new("test-user");

    // Spawn 10 concurrent resolve_with_refresh calls
    let mut handles = Vec::with_capacity(10);
    for _ in 0..10 {
        let r = Arc::clone(&resolver);
        let c = ctx.clone();
        handles.push(tokio::spawn(async move {
            r.resolve_with_refresh::<ThunderingHerdCredential>("herd-cred", &c)
                .await
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles).await;

    // All 10 tasks should succeed
    for (i, result) in results.iter().enumerate() {
        let inner = result.as_ref().expect("task should not panic");
        assert!(inner.is_ok(), "task {i} failed: {inner:?}");
    }

    // Verify all callers got the refreshed token
    for result in &results {
        let handle = result.as_ref().unwrap().as_ref().unwrap();
        let value = handle.snapshot().expose().expose_secret(|s| s.to_owned());
        assert_eq!(value, "refreshed-token");
    }

    // The critical invariant: only 1 refresh should have happened
    assert_eq!(
        REFRESH_COUNT.load(Ordering::SeqCst),
        1,
        "expected exactly 1 refresh, but {} occurred",
        REFRESH_COUNT.load(Ordering::SeqCst)
    );
}
