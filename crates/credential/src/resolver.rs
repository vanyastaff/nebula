//! Runtime credential resolution.
//!
//! Loads stored credentials, deserializes state, and projects to
//! [`AuthScheme`](nebula_core::AuthScheme) via the
//! [`Credential::project()`](crate::credential::Credential::project) pipeline.
//!
//! For refreshable credentials, use [`CredentialResolver::resolve_with_refresh()`]
//! which coordinates refresh via [`RefreshCoordinator`] to prevent
//! thundering herd.

use std::sync::Arc;

use crate::context::CredentialContext;
use crate::credential::Credential;
use crate::handle::CredentialHandle;
use crate::refresh::{RefreshAttempt, RefreshCoordinator};
use crate::resolve::RefreshOutcome;
use crate::state::CredentialState;
use crate::store::{CredentialStore, PutMode, StoreError, StoredCredential};

/// Resolves credentials from storage into typed [`CredentialHandle`]s.
///
/// The resolver loads a [`StoredCredential`](crate::store::StoredCredential),
/// verifies the `state_kind` matches the expected credential type,
/// deserializes the state, and projects it to the [`AuthScheme`](nebula_core::AuthScheme).
///
/// For refreshable credentials, [`resolve_with_refresh()`](Self::resolve_with_refresh)
/// coordinates concurrent refresh attempts through the embedded
/// [`RefreshCoordinator`].
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::{CredentialResolver, InMemoryStore};
/// use nebula_credential::credentials::ApiKeyCredential;
///
/// let store = Arc::new(InMemoryStore::new());
/// let resolver = CredentialResolver::new(store);
///
/// let handle = resolver.resolve::<ApiKeyCredential>("my-api-key").await?;
/// let token = handle.snapshot();
/// ```
pub struct CredentialResolver<S: CredentialStore> {
    store: Arc<S>,
    refresh_coordinator: RefreshCoordinator,
}

impl<S: CredentialStore> CredentialResolver<S> {
    /// Creates a new resolver backed by the given store.
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            refresh_coordinator: RefreshCoordinator::new(),
        }
    }

    /// Resolves a credential by ID into a typed handle.
    ///
    /// Loads the stored credential, deserializes the state, and
    /// projects it to the [`AuthScheme`](nebula_core::AuthScheme) via
    /// [`Credential::project()`](crate::credential::Credential::project).
    ///
    /// # Errors
    ///
    /// Returns [`ResolveError::Store`] if the credential is not found
    /// or the store is unavailable.
    ///
    /// Returns [`ResolveError::KindMismatch`] if the stored `state_kind`
    /// does not match `C::State::KIND`.
    ///
    /// Returns [`ResolveError::Deserialize`] if the stored bytes cannot
    /// be deserialized into `C::State`.
    pub async fn resolve<C>(
        &self,
        credential_id: &str,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Credential,
    {
        let stored = self.load_and_verify::<C>(credential_id).await?;
        let state: C::State = self.deserialize::<C>(credential_id, &stored)?;
        let scheme = C::project(&state);
        Ok(CredentialHandle::new(scheme, credential_id))
    }

    /// Resolves a credential, refreshing it before expiry.
    ///
    /// If the credential's remaining lifetime is within the
    /// [`RefreshPolicy::early_refresh`](crate::resolve::RefreshPolicy::early_refresh)
    /// window (default 5 minutes), the resolver proactively triggers a
    /// refresh through the embedded [`RefreshCoordinator`]. Only one
    /// caller performs the actual refresh; others wait and then re-read
    /// the updated state from the store.
    ///
    /// For non-expiring credentials this behaves identically to
    /// [`resolve()`](Self::resolve).
    ///
    /// # Errors
    ///
    /// Returns all errors from [`resolve()`](Self::resolve), plus:
    ///
    /// - [`ResolveError::Refresh`] if `Credential::refresh()` fails.
    /// - [`ResolveError::Store`] if the CAS write after refresh fails.
    /// - [`ResolveError::ReauthRequired`] if the refresh indicates
    ///   the credential needs full re-authentication.
    pub async fn resolve_with_refresh<C>(
        &self,
        credential_id: &str,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Credential,
    {
        let stored = self.load_and_verify::<C>(credential_id).await?;
        let state: C::State = self.deserialize::<C>(credential_id, &stored)?;

        // Refresh proactively before expiry per REFRESH_POLICY.early_refresh,
        // with random jitter to prevent thundering herd across credentials
        // that share the same expiry window.
        let needs_refresh = state.expires_at().is_some_and(|exp| {
            let now = chrono::Utc::now();
            let jitter = if C::REFRESH_POLICY.jitter > std::time::Duration::ZERO {
                let bound = C::REFRESH_POLICY.jitter.as_millis() as u64;
                std::time::Duration::from_millis(rand::random_range(0..bound))
            } else {
                std::time::Duration::ZERO
            };
            let early_with_jitter = C::REFRESH_POLICY.early_refresh + jitter;
            let early =
                chrono::Duration::from_std(early_with_jitter).unwrap_or(chrono::Duration::zero());
            exp - now <= early
        });

        if !needs_refresh {
            let scheme = C::project(&state);
            return Ok(CredentialHandle::new(scheme, credential_id));
        }

        // Circuit breaker: skip refresh if too many recent failures
        if self.refresh_coordinator.is_circuit_open(credential_id) {
            tracing::warn!(
                credential_id,
                "circuit breaker open: too many refresh failures, serving potentially stale credential"
            );
            let scheme = C::project(&state);
            return Ok(CredentialHandle::new(scheme, credential_id));
        }

        // Coordinate refresh -- only one caller does the work
        match self.refresh_coordinator.try_refresh(credential_id) {
            RefreshAttempt::Winner(notify) => {
                // Acquire a global concurrency permit BEFORE the refresh HTTP
                // call. This prevents 429 cascades when many credentials expire
                // simultaneously. The permit drops when this block exits (on
                // success, error, or panic), freeing a slot for the next caller.
                let _permit = self.refresh_coordinator.acquire_permit().await;

                // scopeguard: always clean up in-flight entry and notify waiters,
                // even on panic/timeout. Both complete() and notify_waiters() are
                // sync, so they're safe to call from Drop (B8 fix).
                let credential_id_for_guard = credential_id.to_string();
                let coordinator = &self.refresh_coordinator;
                let _guard = scopeguard::guard(notify, |n| {
                    coordinator.complete(&credential_id_for_guard);
                    n.notify_waiters();
                });
                let result = self
                    .perform_refresh::<C>(credential_id, state, stored, ctx)
                    .await;
                // Track success/failure for circuit breaker
                if result.is_ok() {
                    self.refresh_coordinator.record_success(credential_id);
                } else {
                    self.refresh_coordinator.record_failure(credential_id);
                }
                // complete() and notify_waiters() called by guard on drop
                result
            }
            RefreshAttempt::Waiter(notify) => {
                // 60s max wait -- don't hang forever if winner is slow
                match tokio::time::timeout(std::time::Duration::from_secs(60), notify.notified())
                    .await
                {
                    Ok(()) => {}
                    Err(_) => {
                        tracing::warn!(
                            credential_id,
                            "refresh waiter timed out after 60s, re-reading from store"
                        );
                    }
                }
                // Re-read from store regardless (winner may have updated)
                self.resolve::<C>(credential_id).await
            }
        }
    }

    /// Returns a reference to the embedded [`RefreshCoordinator`].
    ///
    /// Primarily useful for testing and diagnostics.
    pub fn refresh_coordinator(&self) -> &RefreshCoordinator {
        &self.refresh_coordinator
    }

    /// Loads and verifies a stored credential's `state_kind`.
    async fn load_and_verify<C>(
        &self,
        credential_id: &str,
    ) -> Result<StoredCredential, ResolveError>
    where
        C: Credential,
    {
        let stored = self
            .store
            .get(credential_id)
            .await
            .map_err(ResolveError::Store)?;

        let expected_kind = <C::State as CredentialState>::KIND;
        if stored.state_kind != expected_kind {
            return Err(ResolveError::KindMismatch {
                credential_id: credential_id.to_string(),
                expected: expected_kind.to_string(),
                actual: stored.state_kind,
            });
        }

        Ok(stored)
    }

    /// Deserializes stored bytes into the credential state type.
    fn deserialize<C>(
        &self,
        credential_id: &str,
        stored: &StoredCredential,
    ) -> Result<C::State, ResolveError>
    where
        C: Credential,
    {
        serde_json::from_slice(&stored.data).map_err(|e| ResolveError::Deserialize {
            credential_id: credential_id.to_string(),
            reason: e.to_string(),
        })
    }

    /// Performs the actual refresh: calls `C::refresh()`, writes back
    /// to the store with CAS, and projects the result.
    async fn perform_refresh<C>(
        &self,
        credential_id: &str,
        mut state: C::State,
        stored: StoredCredential,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Credential,
    {
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            C::refresh(&mut state, ctx),
        )
        .await
        .map_err(|_| ResolveError::Refresh {
            credential_id: credential_id.to_string(),
            reason: "framework timeout: refresh took longer than 30s".to_string(),
        })?
        .map_err(|e| ResolveError::Refresh {
            credential_id: credential_id.to_string(),
            reason: e.to_string(),
        })?;

        match outcome {
            RefreshOutcome::Refreshed => {
                let data = serde_json::to_vec(&state).map_err(|e| ResolveError::Refresh {
                    credential_id: credential_id.to_string(),
                    reason: format!("failed to serialize refreshed state: {e}"),
                })?;

                // CAS retry loop: if another writer bumped the version, retry
                // with the actual version. The refreshed `data` is reused —
                // we must NOT call C::refresh() again because the old refresh
                // token may already be invalidated (OAuth2 single-use tokens).
                let mut current_version = stored.version;
                for _attempt in 0..3 {
                    let updated = StoredCredential {
                        data: data.clone(),
                        updated_at: chrono::Utc::now(),
                        expires_at: state.expires_at(),
                        ..stored.clone()
                    };
                    match self
                        .store
                        .put(
                            updated,
                            PutMode::CompareAndSwap {
                                expected_version: current_version,
                            },
                        )
                        .await
                    {
                        Ok(_) => {
                            let scheme = C::project(&state);
                            return Ok(CredentialHandle::new(scheme, credential_id));
                        }
                        Err(StoreError::VersionConflict { actual, .. }) => {
                            tracing::warn!(
                                credential_id,
                                expected = current_version,
                                actual,
                                "CAS conflict on refresh write, retrying with same token"
                            );
                            current_version = actual;
                            continue;
                        }
                        Err(e) => return Err(ResolveError::Store(e)),
                    }
                }
                // All retries exhausted
                Err(ResolveError::Store(StoreError::VersionConflict {
                    id: credential_id.to_string(),
                    expected: current_version,
                    actual: current_version,
                }))
            }
            RefreshOutcome::NotSupported => {
                // Not refreshable -- return current state anyway
                let scheme = C::project(&state);
                Ok(CredentialHandle::new(scheme, credential_id))
            }
            RefreshOutcome::ReauthRequired => Err(ResolveError::ReauthRequired {
                credential_id: credential_id.to_string(),
            }),
        }
    }
}

/// Error during credential resolution.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// Credential not found or store error.
    #[error("store error: {0}")]
    Store(#[from] StoreError),

    /// State kind mismatch between stored and expected.
    #[error("credential {credential_id}: expected kind {expected}, found {actual}")]
    KindMismatch {
        /// The credential ID.
        credential_id: String,
        /// The expected `state_kind`.
        expected: String,
        /// The actual `state_kind` found in storage.
        actual: String,
    },

    /// Failed to deserialize stored state.
    #[error("credential {credential_id}: deserialize failed: {reason}")]
    Deserialize {
        /// The credential ID.
        credential_id: String,
        /// The deserialization error message.
        reason: String,
    },

    /// Refresh failed.
    #[error("credential {credential_id}: refresh failed: {reason}")]
    Refresh {
        /// The credential ID.
        credential_id: String,
        /// The refresh error message.
        reason: String,
    },

    /// Credential needs full re-authentication (refresh token expired).
    #[error("credential {credential_id}: re-authentication required")]
    ReauthRequired {
        /// The credential ID.
        credential_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::ApiKeyCredential;
    use crate::store::{PutMode, StoredCredential};
    use crate::store_memory::InMemoryStore;

    // ── Test credential for early refresh ──────────────────────────────

    use crate::SecretString;
    use crate::context::CredentialContext;
    use crate::credential::Credential;
    use crate::description::CredentialDescription;
    use crate::error::CredentialError;
    use crate::pending::NoPendingState;
    use crate::resolve::{RefreshOutcome, RefreshPolicy, StaticResolveResult};
    use crate::scheme::BearerToken;
    use nebula_parameter::ParameterCollection;
    use nebula_parameter::values::ParameterValues;

    /// State that reports an expiration time.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct ExpiringState {
        token: String,
        expires_at: chrono::DateTime<chrono::Utc>,
    }

    impl CredentialState for ExpiringState {
        const KIND: &'static str = "expiring_test";
        const VERSION: u32 = 1;

        fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
            Some(self.expires_at)
        }
    }

    /// A test credential that is refreshable and uses `ExpiringState`.
    struct RefreshableTestCredential;

    impl Credential for RefreshableTestCredential {
        type Scheme = BearerToken;
        type State = ExpiringState;
        type Pending = NoPendingState;

        const KEY: &'static str = "refreshable_test";
        const REFRESHABLE: bool = true;
        const REFRESH_POLICY: RefreshPolicy = RefreshPolicy {
            early_refresh: std::time::Duration::from_secs(300), // 5 minutes
            ..RefreshPolicy::DEFAULT
        };

        fn description() -> CredentialDescription {
            CredentialDescription {
                key: Self::KEY.to_owned(),
                name: "Refreshable Test".to_owned(),
                description: "Test credential for early refresh".to_owned(),
                icon: None,
                icon_url: None,
                documentation_url: None,
                properties: Self::parameters(),
            }
        }

        fn parameters() -> ParameterCollection {
            ParameterCollection::new()
        }

        fn project(state: &ExpiringState) -> BearerToken {
            BearerToken::new(SecretString::new(state.token.clone()))
        }

        async fn resolve(
            _values: &ParameterValues,
            _ctx: &CredentialContext,
        ) -> Result<StaticResolveResult<ExpiringState>, CredentialError> {
            unreachable!("not used in refresh tests")
        }

        async fn refresh(
            state: &mut ExpiringState,
            _ctx: &CredentialContext,
        ) -> Result<RefreshOutcome, CredentialError> {
            // Simulate a successful refresh: new token, new expiry
            state.token = "refreshed-token".to_owned();
            state.expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
            Ok(RefreshOutcome::Refreshed)
        }
    }

    #[tokio::test]
    async fn resolve_api_key_credential() {
        let store = Arc::new(InMemoryStore::new());

        // Construct raw JSON directly because SecretString serializes
        // as "[REDACTED]" — the real store holds encrypted raw values.
        let data = br#"{"token":"test-api-key"}"#.to_vec();

        let cred = StoredCredential {
            id: "my-api-key".into(),
            credential_key: "api_key".into(),
            data,
            state_kind: "bearer".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let resolver = CredentialResolver::new(store);
        let handle = resolver
            .resolve::<ApiKeyCredential>("my-api-key")
            .await
            .unwrap();

        let snapshot = handle.snapshot();
        let value = snapshot.expose().expose_secret(|s| s.to_owned());
        assert_eq!(value, "test-api-key");
        assert_eq!(handle.credential_id(), "my-api-key");
    }

    #[tokio::test]
    async fn resolve_not_found() {
        let store = Arc::new(InMemoryStore::new());
        let resolver = CredentialResolver::new(store);

        let result = resolver.resolve::<ApiKeyCredential>("nonexistent").await;
        assert!(matches!(
            result,
            Err(ResolveError::Store(StoreError::NotFound { .. }))
        ));
    }

    #[tokio::test]
    async fn resolve_kind_mismatch() {
        let store = Arc::new(InMemoryStore::new());

        let cred = StoredCredential {
            id: "wrong-kind".into(),
            credential_key: "database".into(),
            data: b"{}".to_vec(),
            state_kind: "database_auth".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let resolver = CredentialResolver::new(store);
        let result = resolver.resolve::<ApiKeyCredential>("wrong-kind").await;
        assert!(matches!(result, Err(ResolveError::KindMismatch { .. })));
    }

    #[tokio::test]
    async fn resolve_deserialize_failure() {
        let store = Arc::new(InMemoryStore::new());

        let cred = StoredCredential {
            id: "bad-data".into(),
            credential_key: "api_key".into(),
            data: b"not-valid-json".to_vec(),
            state_kind: "bearer".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let resolver = CredentialResolver::new(store);
        let result = resolver.resolve::<ApiKeyCredential>("bad-data").await;
        assert!(matches!(result, Err(ResolveError::Deserialize { .. })));
    }

    #[tokio::test]
    async fn early_refresh_triggers_before_expiry() {
        let store = Arc::new(InMemoryStore::new());

        // Token expires in 4 minutes -- inside the 5-minute early_refresh window
        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(4);
        let state = ExpiringState {
            token: "old-token".to_owned(),
            expires_at,
        };
        let data = serde_json::to_vec(&state).unwrap();

        let cred = StoredCredential {
            id: "expiring-cred".into(),
            credential_key: "refreshable_test".into(),
            data,
            state_kind: "expiring_test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: Some(expires_at),
            metadata: Default::default(),
        };
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let resolver = CredentialResolver::new(store);
        let ctx = CredentialContext::new("test-user");

        let handle = resolver
            .resolve_with_refresh::<RefreshableTestCredential>("expiring-cred", &ctx)
            .await
            .unwrap();

        // The refresh should have fired because 4 min < 5 min early_refresh
        let value = handle.snapshot().expose().expose_secret(|s| s.to_owned());
        assert_eq!(value, "refreshed-token");
    }

    // ── CAS retry test infrastructure ───────────────────────────────

    use std::sync::atomic::{AtomicU32, Ordering};

    /// A store wrapper that injects a configurable number of CAS
    /// `VersionConflict` errors before delegating to the inner store.
    struct ConflictingStore {
        inner: InMemoryStore,
        /// How many CAS puts should fail before succeeding.
        remaining_conflicts: AtomicU32,
    }

    impl ConflictingStore {
        fn new(conflicts: u32) -> Self {
            Self {
                inner: InMemoryStore::new(),
                remaining_conflicts: AtomicU32::new(conflicts),
            }
        }
    }

    impl CredentialStore for ConflictingStore {
        async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
            self.inner.get(id).await
        }

        async fn put(
            &self,
            credential: StoredCredential,
            mode: PutMode,
        ) -> Result<StoredCredential, StoreError> {
            // Only inject conflicts on CAS puts
            if let PutMode::CompareAndSwap { expected_version } = &mode {
                let remaining = self.remaining_conflicts.load(Ordering::SeqCst);
                if remaining > 0 {
                    self.remaining_conflicts.fetch_sub(1, Ordering::SeqCst);
                    // Bump the real version in the inner store via Overwrite
                    // so the next CAS attempt sees the new version.
                    let bumped = self.inner.put(credential, PutMode::Overwrite).await?;
                    return Err(StoreError::VersionConflict {
                        id: bumped.id,
                        expected: *expected_version,
                        actual: bumped.version,
                    });
                }
            }
            self.inner.put(credential, mode).await
        }

        async fn delete(&self, id: &str) -> Result<(), StoreError> {
            self.inner.delete(id).await
        }

        async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
            self.inner.list(state_kind).await
        }

        async fn exists(&self, id: &str) -> Result<bool, StoreError> {
            self.inner.exists(id).await
        }
    }

    /// A test credential that counts how many times `refresh()` is called,
    /// to verify the CAS retry does NOT re-invoke refresh.
    static CAS_REFRESH_COUNT: AtomicU32 = AtomicU32::new(0);

    struct CasRetryTestCredential;

    impl Credential for CasRetryTestCredential {
        type Scheme = BearerToken;
        type State = ExpiringState;
        type Pending = NoPendingState;

        const KEY: &'static str = "cas_retry_test";
        const REFRESHABLE: bool = true;
        const REFRESH_POLICY: RefreshPolicy = RefreshPolicy {
            early_refresh: std::time::Duration::from_secs(300),
            jitter: std::time::Duration::ZERO,
            ..RefreshPolicy::DEFAULT
        };

        fn description() -> CredentialDescription {
            CredentialDescription {
                key: Self::KEY.to_owned(),
                name: "CAS Retry Test".to_owned(),
                description: "Test credential for CAS retry".to_owned(),
                icon: None,
                icon_url: None,
                documentation_url: None,
                properties: Self::parameters(),
            }
        }

        fn parameters() -> ParameterCollection {
            ParameterCollection::new()
        }

        fn project(state: &ExpiringState) -> BearerToken {
            BearerToken::new(SecretString::new(state.token.clone()))
        }

        async fn resolve(
            _values: &ParameterValues,
            _ctx: &CredentialContext,
        ) -> Result<StaticResolveResult<ExpiringState>, CredentialError> {
            unreachable!("not used in CAS retry tests")
        }

        async fn refresh(
            state: &mut ExpiringState,
            _ctx: &CredentialContext,
        ) -> Result<RefreshOutcome, CredentialError> {
            CAS_REFRESH_COUNT.fetch_add(1, Ordering::SeqCst);
            state.token = "refreshed-token".to_owned();
            state.expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
            Ok(RefreshOutcome::Refreshed)
        }
    }

    #[tokio::test]
    async fn cas_retry_succeeds_after_version_conflict() {
        CAS_REFRESH_COUNT.store(0, Ordering::SeqCst);

        // 1 CAS conflict before success
        let store = Arc::new(ConflictingStore::new(1));

        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(2);
        let state = ExpiringState {
            token: "old-token".to_owned(),
            expires_at,
        };
        let data = serde_json::to_vec(&state).unwrap();

        let cred = StoredCredential {
            id: "cas-cred".into(),
            credential_key: "cas_retry_test".into(),
            data,
            state_kind: "expiring_test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: Some(expires_at),
            metadata: Default::default(),
        };
        store.inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let resolver = CredentialResolver::new(store);
        let ctx = CredentialContext::new("test-user");

        let handle = resolver
            .resolve_with_refresh::<CasRetryTestCredential>("cas-cred", &ctx)
            .await
            .unwrap();

        // Token should be the refreshed value despite CAS conflict
        let value = handle.snapshot().expose().expose_secret(|s| s.to_owned());
        assert_eq!(value, "refreshed-token");

        // Critical: refresh() must be called exactly once — the retry
        // reuses the same token, it does NOT re-invoke the provider.
        assert_eq!(
            CAS_REFRESH_COUNT.load(Ordering::SeqCst),
            1,
            "refresh() should be called exactly once, CAS retry reuses the token"
        );
    }

    #[tokio::test]
    async fn cas_retry_exhausted_returns_version_conflict() {
        CAS_REFRESH_COUNT.store(0, Ordering::SeqCst);

        // 5 CAS conflicts — more than the 3-attempt limit
        let store = Arc::new(ConflictingStore::new(5));

        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(2);
        let state = ExpiringState {
            token: "old-token".to_owned(),
            expires_at,
        };
        let data = serde_json::to_vec(&state).unwrap();

        let cred = StoredCredential {
            id: "cas-exhausted".into(),
            credential_key: "cas_retry_test".into(),
            data,
            state_kind: "expiring_test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: Some(expires_at),
            metadata: Default::default(),
        };
        store.inner.put(cred, PutMode::CreateOnly).await.unwrap();

        let resolver = CredentialResolver::new(store);
        let ctx = CredentialContext::new("test-user");

        let result = resolver
            .resolve_with_refresh::<CasRetryTestCredential>("cas-exhausted", &ctx)
            .await;

        // Should fail with VersionConflict after exhausting retries
        assert!(
            matches!(
                result,
                Err(ResolveError::Store(StoreError::VersionConflict { .. }))
            ),
            "expected VersionConflict after retries exhausted, got: {result:?}"
        );

        // refresh() still called exactly once
        assert_eq!(CAS_REFRESH_COUNT.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn no_early_refresh_when_outside_window() {
        let store = Arc::new(InMemoryStore::new());

        // Token expires in 10 minutes -- outside the 5-minute early_refresh window
        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(10);
        let state = ExpiringState {
            token: "still-valid-token".to_owned(),
            expires_at,
        };
        let data = serde_json::to_vec(&state).unwrap();

        let cred = StoredCredential {
            id: "valid-cred".into(),
            credential_key: "refreshable_test".into(),
            data,
            state_kind: "expiring_test".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: Some(expires_at),
            metadata: Default::default(),
        };
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let resolver = CredentialResolver::new(store);
        let ctx = CredentialContext::new("test-user");

        let handle = resolver
            .resolve_with_refresh::<RefreshableTestCredential>("valid-cred", &ctx)
            .await
            .unwrap();

        // Should NOT have refreshed -- token still valid outside the window
        let value = handle.snapshot().expose().expose_secret(|s| s.to_owned());
        assert_eq!(value, "still-valid-token");
    }
}
