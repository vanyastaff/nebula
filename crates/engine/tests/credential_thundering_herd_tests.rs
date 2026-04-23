//! Integration test: thundering herd prevention on credential refresh.

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use nebula_credential::{
    Credential, CredentialContext, CredentialMetadata, CredentialStore, NoPendingState,
    SecretString,
    error::CredentialError,
    resolve::{RefreshOutcome, RefreshPolicy, StaticResolveResult},
    scheme::SecretToken,
    store::{PutMode, StoredCredential},
};
use nebula_schema::FieldValues;
use nebula_storage::credential::InMemoryStore;

static REFRESH_COUNT: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ThunderingHerdState {
    token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl nebula_credential::CredentialState for ThunderingHerdState {
    const KIND: &'static str = "thundering_herd_test";
    const VERSION: u32 = 1;

    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        Some(self.expires_at)
    }
}

struct ThunderingHerdCredential;

impl Credential for ThunderingHerdCredential {
    type Input = FieldValues;
    type Scheme = SecretToken;
    type State = ThunderingHerdState;
    type Pending = NoPendingState;

    const KEY: &'static str = "thundering_herd_test";
    const REFRESHABLE: bool = true;
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy {
        early_refresh: std::time::Duration::from_mins(5),
        jitter: std::time::Duration::ZERO,
        ..RefreshPolicy::DEFAULT
    };

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::new(
            nebula_core::credential_key!("thundering_herd_test"),
            "Thundering Herd Test",
            "Test credential for thundering herd prevention",
            Self::schema(),
            nebula_credential::AuthPattern::SecretToken,
        )
    }

    fn project(state: &ThunderingHerdState) -> SecretToken {
        SecretToken::new(SecretString::new(state.token.clone()))
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<ThunderingHerdState>, CredentialError> {
        unreachable!("not used in thundering herd tests")
    }

    async fn refresh(
        state: &mut ThunderingHerdState,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        REFRESH_COUNT.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        state.token = "refreshed-token".to_owned();
        state.expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
        Ok(RefreshOutcome::Refreshed)
    }
}

#[tokio::test]
async fn only_one_refresh_under_concurrent_access() {
    REFRESH_COUNT.store(0, Ordering::SeqCst);

    let store = Arc::new(InMemoryStore::new());
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

    let resolver = Arc::new(nebula_engine::credential::CredentialResolver::new(store));
    let ctx = CredentialContext::for_test("test-user");

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

    for (i, result) in results.iter().enumerate() {
        let inner = result.as_ref().expect("task should not panic");
        assert!(inner.is_ok(), "task {i} failed: {inner:?}");
    }

    for result in &results {
        let handle = result.as_ref().unwrap().as_ref().unwrap();
        let value = handle.snapshot().token().expose_secret().to_owned();
        assert_eq!(value, "refreshed-token");
    }

    assert_eq!(REFRESH_COUNT.load(Ordering::SeqCst), 1);
}
