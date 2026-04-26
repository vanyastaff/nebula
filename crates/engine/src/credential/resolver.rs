//! Runtime credential resolution.

use std::sync::Arc;

use nebula_credential::{
    Credential, CredentialContext, CredentialEvent, CredentialHandle, CredentialId,
    CredentialState, Refreshable,
    resolve::RefreshOutcome,
    store::{CredentialStore, PutMode, StoreError, StoredCredential},
};
#[cfg(feature = "rotation")]
use nebula_credential::{
    credentials::{OAuth2Credential, OAuth2State},
    error::CredentialError,
};
use nebula_eventbus::EventBus;

use crate::credential::refresh::{RefreshAttempt, RefreshCoordinator};
#[cfg(feature = "rotation")]
use crate::credential::rotation::refresh_oauth2_state;

/// Runtime credential resolver with optional coordinated refresh.
pub struct CredentialResolver<S: CredentialStore> {
    store: Arc<S>,
    refresh_coordinator: RefreshCoordinator,
    event_bus: Option<Arc<EventBus<CredentialEvent>>>,
}

impl<S: CredentialStore> CredentialResolver<S> {
    /// Create a resolver backed by the given credential store.
    #[must_use]
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            refresh_coordinator: RefreshCoordinator::new(),
            event_bus: None,
        }
    }

    #[must_use = "builder methods must be chained or built"]
    /// Attach an event bus to emit credential refresh lifecycle events.
    pub fn with_event_bus(mut self, bus: Arc<EventBus<CredentialEvent>>) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// Resolve a credential state into a typed handle.
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

    /// Resolve a credential and refresh it when it enters the early-refresh window.
    ///
    /// Per Tech Spec §15.4 — bound on [`Refreshable`] so a non-refreshable
    /// credential cannot reach this dispatch path. Probe 4
    /// (`compile_fail_engine_dispatch_capability`) cements the structural
    /// barrier with `E0277` at the dispatch site.
    pub async fn resolve_with_refresh<C>(
        &self,
        credential_id: &str,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Refreshable,
    {
        let stored = self.load_and_verify::<C>(credential_id).await?;
        let state: C::State = self.deserialize::<C>(credential_id, &stored)?;

        let needs_refresh = state.expires_at().is_some_and(|exp| {
            let now = chrono::Utc::now();
            let policy = <C as Refreshable>::REFRESH_POLICY;
            let jitter = if policy.jitter > std::time::Duration::ZERO {
                let bound_ms = policy.jitter.as_millis();
                if bound_ms == 0 {
                    std::time::Duration::ZERO
                } else {
                    let upper = u64::try_from(bound_ms).unwrap_or(u64::MAX);
                    std::time::Duration::from_millis(rand::random_range(0..upper))
                }
            } else {
                std::time::Duration::ZERO
            };
            let early_with_jitter = policy.early_refresh + jitter;
            let early =
                chrono::Duration::from_std(early_with_jitter).unwrap_or(chrono::Duration::zero());
            exp - now <= early
        });

        if !needs_refresh {
            let scheme = C::project(&state);
            return Ok(CredentialHandle::new(scheme, credential_id));
        }

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

        match self.refresh_coordinator.try_refresh(credential_id) {
            RefreshAttempt::Winner => {
                let _permit = self.refresh_coordinator.acquire_permit().await;

                let credential_id_for_guard = credential_id.to_string();
                let coordinator = &self.refresh_coordinator;
                let _guard = scopeguard::guard((), |()| {
                    coordinator.complete(&credential_id_for_guard);
                });
                let result = self
                    .perform_refresh::<C>(credential_id, state, stored, ctx)
                    .await;
                if result.is_ok() {
                    self.refresh_coordinator.record_success(credential_id);
                } else {
                    self.refresh_coordinator.record_failure(credential_id);
                }
                result
            },
            RefreshAttempt::Waiter(rx) => {
                if tokio::time::timeout(std::time::Duration::from_secs(5), rx)
                    .await
                    .is_err()
                {
                    tracing::debug!(
                        credential_id,
                        "refresh waiter timed out after 5s, re-reading from store"
                    );
                }
                self.resolve::<C>(credential_id).await
            },
        }
    }

    /// Access the refresh coordinator used by this resolver.
    pub fn refresh_coordinator(&self) -> &RefreshCoordinator {
        &self.refresh_coordinator
    }

    fn emit_refreshed(&self, credential_id: CredentialId) {
        if let Some(bus) = &self.event_bus {
            let _ = bus.emit(CredentialEvent::Refreshed { credential_id });
        }
    }

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

    async fn perform_refresh<C>(
        &self,
        credential_id: &str,
        mut state: C::State,
        stored: StoredCredential,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Refreshable,
    {
        #[cfg(feature = "rotation")]
        async fn try_engine_oauth2_refresh<C: Refreshable>(
            state: &mut C::State,
        ) -> Result<Option<RefreshOutcome>, CredentialError> {
            if C::KEY != OAuth2Credential::KEY {
                return Ok(None);
            }

            let mut oauth_state: OAuth2State =
                serde_json::from_value(serde_json::to_value(&*state).map_err(|e| {
                    CredentialError::Provider(format!(
                        "oauth2 refresh state serialization failed: {e}"
                    ))
                })?)
                .map_err(|e| {
                    CredentialError::Provider(format!("oauth2 refresh state decode failed: {e}"))
                })?;

            refresh_oauth2_state(&mut oauth_state)
                .await
                .map_err(|e| CredentialError::Provider(e.to_string()))?;

            *state = serde_json::from_value(serde_json::to_value(oauth_state).map_err(|e| {
                CredentialError::Provider(format!("oauth2 refresh state serialization failed: {e}"))
            })?)
            .map_err(|e| {
                CredentialError::Provider(format!("oauth2 refresh state encode failed: {e}"))
            })?;

            Ok(Some(RefreshOutcome::Refreshed))
        }

        let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            #[cfg(feature = "rotation")]
            if let Some(outcome) = try_engine_oauth2_refresh::<C>(&mut state).await? {
                return Ok(outcome);
            }
            <C as Refreshable>::refresh(&mut state, ctx).await
        })
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
                            if let Ok(id) = CredentialId::parse(credential_id) {
                                self.emit_refreshed(id);
                            } else {
                                tracing::warn!(
                                    credential_id,
                                    "credential ID is not a valid `CredentialId` (expected `cred_<ULID>`), refresh event not emitted",
                                );
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
                Err(ResolveError::Store(StoreError::VersionConflict {
                    id: credential_id.to_string(),
                    expected: current_version,
                    actual: current_version,
                }))
            },
            RefreshOutcome::NotSupported => {
                let scheme = C::project(&state);
                Ok(CredentialHandle::new(scheme, credential_id))
            },
            RefreshOutcome::ReauthRequired => Err(ResolveError::ReauthRequired {
                credential_id: credential_id.to_string(),
            }),
            _ => {
                let scheme = C::project(&state);
                Ok(CredentialHandle::new(scheme, credential_id))
            },
        }
    }
}

/// Errors produced by [`CredentialResolver`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// Backing credential store operation failed.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    /// Stored state kind does not match the credential state type.
    #[error("credential {credential_id}: expected kind {expected}, found {actual}")]
    KindMismatch {
        /// Credential identifier.
        credential_id: String,
        /// Expected state kind.
        expected: String,
        /// Actual state kind from storage.
        actual: String,
    },
    /// Stored state bytes failed deserialization.
    #[error("credential {credential_id}: deserialize failed: {reason}")]
    Deserialize {
        /// Credential identifier.
        credential_id: String,
        /// Deserialization error message.
        reason: String,
    },
    /// Refresh path failed.
    #[error("credential {credential_id}: refresh failed: {reason}")]
    Refresh {
        /// Credential identifier.
        credential_id: String,
        /// Refresh error message.
        reason: String,
    },
    /// Credential requires full re-authentication.
    #[error("credential {credential_id}: re-authentication required")]
    ReauthRequired {
        /// Credential identifier.
        credential_id: String,
    },
}
