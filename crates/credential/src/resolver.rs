//! Runtime credential resolution.
//!
//! Loads stored credentials, deserializes state, and projects to
//! AuthScheme via the credential projection pipeline.
//!
//! For refreshable credentials, use resolve_with_refresh()
//! which coordinates refresh via RefreshCoordinator to prevent
//! thundering herd.

use std::sync::Arc;

use nebula_core::{CredentialEvent, CredentialId};
use nebula_eventbus::EventBus;

use crate::{
    context::CredentialContext,
    credential::Credential,
    handle::CredentialHandle,
    refresh::{RefreshAttempt, RefreshCoordinator},
    resolve::RefreshOutcome,
    state::CredentialState,
    store::{CredentialStore, PutMode, StoreError, StoredCredential},
};

/// Resolves credentials from storage into typed CredentialHandles.
///
/// The resolver loads a StoredCredential,
/// verifies the `state_kind` matches the expected credential type,
/// deserializes the state, and projects it to the AuthScheme.
///
/// For refreshable credentials, resolve_with_refresh()
/// coordinates concurrent refresh attempts through the embedded
/// RefreshCoordinator.
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
    event_bus: Option<Arc<EventBus<CredentialEvent>>>,
}

impl<S: CredentialStore> CredentialResolver<S> {
    /// Creates a new resolver backed by the given store.
    #[must_use]
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            refresh_coordinator: RefreshCoordinator::new(),
            event_bus: None,
        }
    }

    /// Attaches an event bus for credential lifecycle notifications.
    ///
    /// When set, the resolver emits [`CredentialEvent::Refreshed`] after
    /// a successful token refresh. Emission is best-effort — failures
    /// are silently ignored per the [`EventBus`] contract.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_event_bus(mut self, bus: Arc<EventBus<CredentialEvent>>) -> Self {
        self.event_bus = Some(bus);
        self
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
    /// - [`ResolveError::ReauthRequired`] if the refresh indicates the credential needs full
    ///   re-authentication.
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
                let bound_ms = C::REFRESH_POLICY.jitter.as_millis();
                if bound_ms == 0 {
                    // Sub-millisecond jitter is valid config; treat as no jitter
                    // instead of panicking on an empty random range.
                    std::time::Duration::ZERO
                } else {
                    let upper = u64::try_from(bound_ms).unwrap_or(u64::MAX);
                    std::time::Duration::from_millis(rand::random_range(0..upper))
                }
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

        // Circuit breaker: skip refresh if too many recent failures.
        //
        // Fail-fast fix (issue #258): distinguish "refresh is proactive"
        // from "token is genuinely expired". Serving a stale-but-valid
        // token while the circuit is open is the documented graceful-
        // degradation path. Serving a token that has already passed
        // its real `expires_at` is NOT — it is guaranteed to fail at
        // the remote API and the failure will be misattributed
        // ("the provider rejected our credential") instead of
        // surfacing the real cause ("the refresh circuit is open").
        // Return `ResolveError::Refresh` when the token is past its
        // true expiry so callers can react immediately.
        if self.refresh_coordinator.is_circuit_open(credential_id) {
            let now = chrono::Utc::now();
            let truly_expired = state.expires_at().is_some_and(|exp| exp <= now);
            if truly_expired {
                tracing::warn!(
                    credential_id,
                    "circuit breaker open and token has passed its expiry; failing fast"
                );
                return Err(ResolveError::Refresh {
                    credential_id: credential_id.to_string(),
                    reason: "refresh circuit breaker open and token is expired".to_string(),
                });
            }
            tracing::warn!(
                credential_id,
                "circuit breaker open: too many refresh failures, serving stale-but-valid credential within early-refresh window"
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
            },
            RefreshAttempt::Waiter(notify) => {
                // Race note: `Notify::notify_waiters()` only wakes waiters that
                // are *already registered* at the moment it fires. If the
                // winner completes its refresh faster than this waiter can
                // poll `notify.notified()` for the first time, the wakeup is
                // lost and we would stall until timeout.
                //
                // Mitigations:
                // 1. Eagerly construct and `enable()` the `Notified` future — this registers the
                //    waiter immediately, narrowing the race window from "await-first-poll" down to
                //    "the handful of instructions between returning from try_refresh and enable()".
                // 2. Short (5 s) timeout — the post-wait `resolve` re-read always fetches the fresh
                //    value from the store, so the timeout is not fatal. Staying on 60 s meant a
                //    lost wakeup produced a 60-second latency spike. 5 s bounds worst-case waiter
                //    latency to a value humans still tolerate while leaving room for a slow
                //    legitimate refresh (which itself holds a `refresh_semaphore` permit and is
                //    normally sub-second).
                let notified = notify.notified();
                tokio::pin!(notified);
                // Pre-register before any await so a concurrent
                // `notify_waiters()` will see us.
                notified.as_mut().enable();

                if tokio::time::timeout(std::time::Duration::from_secs(5), notified)
                    .await
                    .is_err()
                {
                    tracing::debug!(
                        credential_id,
                        "refresh waiter did not observe notify within 5s, re-reading from store"
                    );
                }
                // Re-read from store regardless — this is both the
                // normal success path (winner wrote a fresh value) and
                // the race-recovery path (wakeup was lost).
                self.resolve::<C>(credential_id).await
            },
        }
    }

    /// Returns a reference to the embedded [`RefreshCoordinator`].
    ///
    /// Primarily useful for testing and diagnostics.
    pub fn refresh_coordinator(&self) -> &RefreshCoordinator {
        &self.refresh_coordinator
    }

    /// Best-effort emit of a [`CredentialEvent::Refreshed`] event.
    fn emit_refreshed(&self, credential_id: CredentialId) {
        if let Some(bus) = &self.event_bus {
            let _ = bus.emit(CredentialEvent::Refreshed { credential_id });
        }
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
                            match CredentialId::parse(credential_id) {
                                Ok(id) => self.emit_refreshed(id),
                                Err(_) => tracing::warn!(
                                    credential_id,
                                    "credential ID is not a valid UUID, refresh event not emitted",
                                ),
                            }
                            let scheme = C::project(&state);
                            return Ok(CredentialHandle::new(scheme, credential_id));
                        },
                        Err(StoreError::VersionConflict { actual, .. }) => {
                            tracing::warn!(
                                credential_id,
                                expected = current_version,
                                actual,
                                "CAS conflict on refresh write, retrying with same token"
                            );
                            current_version = actual;
                            continue;
                        },
                        Err(e) => return Err(ResolveError::Store(e)),
                    }
                }
                // All retries exhausted
                Err(ResolveError::Store(StoreError::VersionConflict {
                    id: credential_id.to_string(),
                    expected: current_version,
                    actual: current_version,
                }))
            },
            RefreshOutcome::NotSupported => {
                // Not refreshable -- return current state anyway
                let scheme = C::project(&state);
                Ok(CredentialHandle::new(scheme, credential_id))
            },
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
    use nebula_schema::FieldValues;

    use super::*;
    // ── Test credential for early refresh ──────────────────────────────
    use crate::SecretString;
    use crate::{
        context::CredentialContext,
        credential::Credential,
        credentials::ApiKeyCredential,
        error::CredentialError,
        metadata::CredentialMetadata,
        pending::NoPendingState,
        resolve::{RefreshOutcome, RefreshPolicy, StaticResolveResult},
        scheme::SecretToken,
        store::{PutMode, StoredCredential},
        store_memory::InMemoryStore,
    };

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
        type Input = FieldValues;
        type Scheme = SecretToken;
        type State = ExpiringState;
        type Pending = NoPendingState;

        const KEY: &'static str = "refreshable_test";
        const REFRESHABLE: bool = true;
        const REFRESH_POLICY: RefreshPolicy = RefreshPolicy {
            early_refresh: std::time::Duration::from_secs(300), // 5 minutes
            ..RefreshPolicy::DEFAULT
        };

        fn metadata() -> CredentialMetadata {
            CredentialMetadata::new(
                nebula_core::credential_key!("refreshable_test"),
                "Refreshable Test",
                "Test credential for early refresh",
                Self::parameters(),
                nebula_core::AuthPattern::SecretToken,
            )
        }

        fn project(state: &ExpiringState) -> SecretToken {
            SecretToken::new(SecretString::new(state.token.clone()))
        }

        async fn resolve(
            _values: &FieldValues,
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

    /// Same as `RefreshableTestCredential` but with sub-millisecond jitter
    /// to exercise the empty-range panic regression.
    struct TinyJitterRefreshableTestCredential;

    impl Credential for TinyJitterRefreshableTestCredential {
        type Input = FieldValues;
        type Scheme = SecretToken;
        type State = ExpiringState;
        type Pending = NoPendingState;

        const KEY: &'static str = "tiny_jitter_refreshable_test";
        const REFRESHABLE: bool = true;
        const REFRESH_POLICY: RefreshPolicy = RefreshPolicy {
            early_refresh: std::time::Duration::from_secs(300),
            jitter: std::time::Duration::from_micros(500),
            ..RefreshPolicy::DEFAULT
        };

        fn metadata() -> CredentialMetadata {
            CredentialMetadata::new(
                nebula_core::credential_key!("tiny_jitter_refreshable_test"),
                "Tiny Jitter Refreshable Test",
                "Test credential with sub-ms jitter",
                Self::parameters(),
                nebula_core::AuthPattern::SecretToken,
            )
        }

        fn project(state: &ExpiringState) -> SecretToken {
            SecretToken::new(SecretString::new(state.token.clone()))
        }

        async fn resolve(
            _values: &FieldValues,
            _ctx: &CredentialContext,
        ) -> Result<StaticResolveResult<ExpiringState>, CredentialError> {
            unreachable!("not used in refresh tests")
        }

        async fn refresh(
            state: &mut ExpiringState,
            _ctx: &CredentialContext,
        ) -> Result<RefreshOutcome, CredentialError> {
            state.token = "tiny-jitter-refreshed-token".to_owned();
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
            state_kind: "secret_token".into(),
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
        let value = snapshot.token().expose_secret(|s| s.to_owned());
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
            state_kind: "secret_token".into(),
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

    /// Regression for issue #258: when the refresh circuit breaker is
    /// open AND the token is past its real `expires_at`, the resolver
    /// must fail fast with `ResolveError::Refresh` instead of serving
    /// the dead token silently. Otherwise the downstream auth error
    /// looks like "the provider rejected our credential" and the
    /// real cause (refresh circuit open) is invisible to callers.
    #[tokio::test]
    async fn open_circuit_fails_fast_when_token_is_expired() {
        let store = Arc::new(InMemoryStore::new());

        // Token expired 30 seconds ago — well past expiry, not just in
        // the early-refresh window.
        let expires_at = chrono::Utc::now() - chrono::Duration::seconds(30);
        let state = ExpiringState {
            token: "dead-token".to_owned(),
            expires_at,
        };
        let data = serde_json::to_vec(&state).unwrap();

        let cred = StoredCredential {
            id: "expired-cred".into(),
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

        // Trip the circuit breaker by recording 5 failures (the
        // default `CircuitBreakerConfig::failure_threshold`).
        for _ in 0..5 {
            resolver.refresh_coordinator.record_failure("expired-cred");
        }
        assert!(
            resolver.refresh_coordinator.is_circuit_open("expired-cred"),
            "circuit breaker must be open before exercising the fail-fast path"
        );

        let result = resolver
            .resolve_with_refresh::<RefreshableTestCredential>("expired-cred", &ctx)
            .await;
        match result {
            Err(ResolveError::Refresh {
                credential_id,
                reason,
            }) => {
                assert_eq!(credential_id, "expired-cred");
                assert!(
                    reason.contains("circuit") || reason.contains("expired"),
                    "error should explain the circuit-breaker-and-expired cause: {reason}"
                );
            },
            other => panic!(
                "expected ResolveError::Refresh for an expired token under an open circuit, got: {other:?}"
            ),
        }
    }

    /// Regression for issue #258: a non-expiring credential
    /// (`expires_at() == None`) must not be classified as "truly
    /// expired" just because the circuit is open. `is_some_and`
    /// returns false for `None`, and the fail-fast branch is skipped.
    #[tokio::test]
    async fn open_circuit_serves_non_expiring_credential() {
        // ApiKeyCredential returns `expires_at() == None`, so the
        // circuit-breaker-open path must fall through to the graceful
        // serve instead of the fail-fast error.
        let store = Arc::new(InMemoryStore::new());
        let data = br#"{"token":"forever"}"#.to_vec();
        let cred = StoredCredential {
            id: "non-expiring".into(),
            credential_key: "api_key".into(),
            data,
            state_kind: "secret_token".into(),
            state_version: 1,
            version: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: Default::default(),
        };
        store.put(cred, PutMode::CreateOnly).await.unwrap();

        let resolver = CredentialResolver::new(store);
        // Trip the circuit breaker.
        for _ in 0..5 {
            resolver.refresh_coordinator.record_failure("non-expiring");
        }

        // Non-expiring credentials do not take the refresh path at
        // all (they return `needs_refresh = false` before the circuit
        // check), so this just asserts a plain `resolve` on a
        // non-refreshable credential still works under a tripped
        // circuit. Serves as a regression guard that the future
        // behavior of the circuit branch does not start rejecting
        // non-expiring credentials.
        let handle = resolver
            .resolve::<ApiKeyCredential>("non-expiring")
            .await
            .expect("non-expiring credential should resolve under any circuit state");
        let value = handle.snapshot().token().expose_secret(|s| s.to_owned());
        assert_eq!(value, "forever");
    }

    /// Regression for issue #258: with a closed circuit, an expired
    /// token should take the normal refresh path — not the fail-fast
    /// error. Guards against a future refactor that would flip the
    /// conditional and start erroring on every expired token.
    #[tokio::test]
    async fn closed_circuit_expired_token_refreshes_normally() {
        let store = Arc::new(InMemoryStore::new());
        let expires_at = chrono::Utc::now() - chrono::Duration::seconds(30);
        let state = ExpiringState {
            token: "old".to_owned(),
            expires_at,
        };
        let data = serde_json::to_vec(&state).unwrap();

        let cred = StoredCredential {
            id: "closed-circuit-expired".into(),
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
        // Circuit is fresh — no failures recorded.
        assert!(
            !resolver
                .refresh_coordinator
                .is_circuit_open("closed-circuit-expired")
        );

        let handle = resolver
            .resolve_with_refresh::<RefreshableTestCredential>("closed-circuit-expired", &ctx)
            .await
            .expect("closed-circuit + expired = normal refresh path");
        let value = handle.snapshot().token().expose_secret(|s| s.to_owned());
        assert_eq!(value, "refreshed-token");
    }

    /// Complement to `open_circuit_fails_fast_when_token_is_expired`:
    /// a token that is still valid but within the early-refresh window
    /// should still be served gracefully while the circuit is open —
    /// that is the documented graceful-degradation path and must NOT
    /// regress to a hard error.
    #[tokio::test]
    async fn open_circuit_serves_stale_but_valid_token() {
        let store = Arc::new(InMemoryStore::new());

        // Token expires in 2 minutes — inside the 5-minute early
        // refresh window but not yet expired.
        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(2);
        let state = ExpiringState {
            token: "still-valid".to_owned(),
            expires_at,
        };
        let data = serde_json::to_vec(&state).unwrap();

        let cred = StoredCredential {
            id: "soon-expiring".into(),
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

        // Open the circuit.
        for _ in 0..5 {
            resolver.refresh_coordinator.record_failure("soon-expiring");
        }
        assert!(
            resolver
                .refresh_coordinator
                .is_circuit_open("soon-expiring")
        );

        let handle = resolver
            .resolve_with_refresh::<RefreshableTestCredential>("soon-expiring", &ctx)
            .await
            .expect("stale-but-valid token must still resolve while circuit is open");
        let value = handle.snapshot().token().expose_secret(|s| s.to_owned());
        assert_eq!(value, "still-valid");
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
        let value = handle.snapshot().token().expose_secret(|s| s.to_owned());
        assert_eq!(value, "refreshed-token");
    }

    #[tokio::test]
    async fn sub_millisecond_jitter_does_not_panic() {
        let store = Arc::new(InMemoryStore::new());

        // Inside early-refresh window so refresh path runs and computes jitter.
        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(4);
        let state = ExpiringState {
            token: "old-token".to_owned(),
            expires_at,
        };
        let data = serde_json::to_vec(&state).unwrap();

        let cred = StoredCredential {
            id: "tiny-jitter-cred".into(),
            credential_key: "tiny_jitter_refreshable_test".into(),
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
            .resolve_with_refresh::<TinyJitterRefreshableTestCredential>("tiny-jitter-cred", &ctx)
            .await
            .unwrap();

        let value = handle.snapshot().token().expose_secret(|s| s.to_owned());
        assert_eq!(value, "tiny-jitter-refreshed-token");
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
    /// Serializes CAS retry tests that share `CAS_REFRESH_COUNT`.
    ///
    /// `tokio::sync::Mutex` (not `std::sync::Mutex`) — the guard is held
    /// across every `.await` in the test body, which would make the test
    /// future `!Send` with a std mutex and trips `await_holding_lock`.
    static CAS_TEST_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    struct CasRetryTestCredential;

    impl Credential for CasRetryTestCredential {
        type Input = FieldValues;
        type Scheme = SecretToken;
        type State = ExpiringState;
        type Pending = NoPendingState;

        const KEY: &'static str = "cas_retry_test";
        const REFRESHABLE: bool = true;
        const REFRESH_POLICY: RefreshPolicy = RefreshPolicy {
            early_refresh: std::time::Duration::from_secs(300),
            jitter: std::time::Duration::ZERO,
            ..RefreshPolicy::DEFAULT
        };

        fn metadata() -> CredentialMetadata {
            CredentialMetadata::new(
                nebula_core::credential_key!("cas_retry_test"),
                "CAS Retry Test",
                "Test credential for CAS retry",
                Self::parameters(),
                nebula_core::AuthPattern::SecretToken,
            )
        }

        fn project(state: &ExpiringState) -> SecretToken {
            SecretToken::new(SecretString::new(state.token.clone()))
        }

        async fn resolve(
            _values: &FieldValues,
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
        let _guard = CAS_TEST_LOCK.lock().await;
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
        let value = handle.snapshot().token().expose_secret(|s| s.to_owned());
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
        let _guard = CAS_TEST_LOCK.lock().await;
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
        let value = handle.snapshot().token().expose_secret(|s| s.to_owned());
        assert_eq!(value, "still-valid-token");
    }

    #[tokio::test]
    async fn emits_refreshed_event_after_successful_refresh() {
        let store = Arc::new(InMemoryStore::new());
        let cred_id = CredentialId::new();
        let cred_id_str = cred_id.to_string();

        // Token expires in 2 minutes -- inside the 5-minute early_refresh window
        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(2);
        let state = ExpiringState {
            token: "old-token".to_owned(),
            expires_at,
        };
        let data = serde_json::to_vec(&state).unwrap();

        let cred = StoredCredential {
            id: cred_id_str.clone(),
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

        let bus = Arc::new(EventBus::new(16));
        let mut subscriber = bus.subscribe();

        let resolver = CredentialResolver::new(store).with_event_bus(Arc::clone(&bus));
        let ctx = CredentialContext::new("test-user");

        let handle = resolver
            .resolve_with_refresh::<RefreshableTestCredential>(&cred_id_str, &ctx)
            .await
            .unwrap();

        // Refresh should have fired
        let value = handle.snapshot().token().expose_secret(|s| s.to_owned());
        assert_eq!(value, "refreshed-token");

        // Subscriber should have received a Refreshed event
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), subscriber.recv())
            .await
            .expect("timed out waiting for event")
            .expect("bus closed unexpectedly");

        assert_eq!(
            event,
            CredentialEvent::Refreshed {
                credential_id: cred_id,
            }
        );
    }

    #[tokio::test]
    async fn no_event_emitted_when_no_refresh_needed() {
        let store = Arc::new(InMemoryStore::new());

        // Token expires in 10 minutes -- outside the 5-minute early_refresh window
        let expires_at = chrono::Utc::now() + chrono::Duration::minutes(10);
        let state = ExpiringState {
            token: "still-valid".to_owned(),
            expires_at,
        };
        let data = serde_json::to_vec(&state).unwrap();

        let cred = StoredCredential {
            id: "no-event-cred".into(),
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

        let bus = Arc::new(EventBus::new(16));
        let mut subscriber = bus.subscribe();

        let resolver = CredentialResolver::new(store).with_event_bus(Arc::clone(&bus));
        let ctx = CredentialContext::new("test-user");

        let handle = resolver
            .resolve_with_refresh::<RefreshableTestCredential>("no-event-cred", &ctx)
            .await
            .unwrap();

        // Should NOT have refreshed
        let value = handle.snapshot().token().expose_secret(|s| s.to_owned());
        assert_eq!(value, "still-valid");

        // No event should be pending -- recv should time out
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(50), subscriber.recv()).await;
        assert!(result.is_err(), "expected no event, but received one");
    }
}
