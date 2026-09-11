//! Framework executor for credential resolution.

use std::time::Duration;

use crate::{
    Credential, CredentialContext, Interactive, PendingToken,
    error::CredentialError,
    pending_store::{PendingStateStore, PendingStoreError},
    resolve::{InteractionRequest, ResolveResult, StaticResolveResult, UserInput},
};

const CREDENTIAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Outcome of framework-managed credential resolution.
#[derive(Debug)]
#[non_exhaustive]
pub enum ResolveResponse<S> {
    /// Credential is ready and can be persisted by the caller.
    Complete(S),
    /// User interaction is required to continue the flow.
    Pending {
        /// Opaque token for the stored pending state.
        token: PendingToken,
        /// Instruction to present to the caller/UI.
        interaction: InteractionRequest,
    },
    /// Framework should retry continuation after this delay.
    Retry {
        /// Delay before the next poll.
        after: Duration,
        /// Opaque token for the re-stored pending state when retry follows `execute_continue`.
        token: Option<PendingToken>,
    },
}

/// Error during framework-managed resolution.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExecutorError {
    /// Credential operation timed out.
    #[error("credential operation timed out after {timeout:?}")]
    Timeout {
        /// Timeout duration that was exceeded.
        timeout: Duration,
    },
    /// Error returned by credential implementation.
    #[error("credential error: {0}")]
    Credential(#[from] CredentialError),
    /// Error reading/writing pending state.
    #[error("pending store error: {0}")]
    PendingStore(#[from] PendingStoreError),
    /// Caller did not provide a session id; interactive flows require explicit
    /// session scoping in the [`CredentialContext`].
    ///
    /// The pending store is keyed by `(KEY, owner, session)`. If two concurrent
    /// owners both omitted the session id, a silent `"default"` fallback would
    /// collapse them into the same scoping bucket and risk cross-session token
    /// collisions. Fail-closed: callers must populate `session_id` explicitly.
    #[error(
        "credential context missing session_id; interactive flows require explicit session \
         scoping (per Tech Spec §15.4)"
    )]
    MissingSessionId,
    /// A one-shot continuation returned a polling-only outcome after its
    /// pending state had been consumed.
    #[error("one-shot credential continuation returned a polling outcome")]
    InvalidContinuationOutcome,
}

/// Execute base credential resolution with a timeout.
///
/// Properties have already crossed the canonical schema pipeline and trusted
/// typed decoder. This executor neither reconstructs a schema nor fabricates a
/// second proof.
#[tracing::instrument(name = "credential.execute.resolve", skip_all, fields(credential_key = C::KEY))]
pub async fn execute_resolve<C>(
    properties: &C::Properties,
    ctx: &CredentialContext,
) -> Result<ResolveResponse<C::State>, ExecutorError>
where
    C: Credential,
{
    let result = tokio::time::timeout(CREDENTIAL_TIMEOUT, C::resolve(properties, ctx))
        .await
        .map_err(|_| ExecutorError::Timeout {
            timeout: CREDENTIAL_TIMEOUT,
        })?
        .map_err(ExecutorError::Credential)?;

    match result {
        StaticResolveResult::Complete(state) => Ok(ResolveResponse::Complete(state)),
        StaticResolveResult::Retry { after } => Ok(ResolveResponse::Retry { after, token: None }),
    }
}

/// Begin an interactive credential flow and persist any typed pending state.
///
/// # Errors
///
/// Returns [`ExecutorError::MissingSessionId`] before provider code runs when
/// the context cannot bind pending state to an authenticated session. Other
/// failures preserve their typed credential or pending-store category.
#[tracing::instrument(name = "credential.execute.begin", skip_all, fields(credential_key = C::KEY))]
pub async fn execute_begin<C, S>(
    properties: &C::Properties,
    ctx: &CredentialContext,
    pending_store: &S,
) -> Result<ResolveResponse<C::State>, ExecutorError>
where
    C: Interactive,
    S: PendingStateStore,
{
    let session_id = ctx.session_id().ok_or(ExecutorError::MissingSessionId)?;
    let result = tokio::time::timeout(CREDENTIAL_TIMEOUT, C::begin(properties, ctx))
        .await
        .map_err(|_| ExecutorError::Timeout {
            timeout: CREDENTIAL_TIMEOUT,
        })?
        .map_err(ExecutorError::Credential)?;

    match result {
        ResolveResult::Complete(state) => Ok(ResolveResponse::Complete(state)),
        ResolveResult::Pending { state, interaction } => {
            let token = pending_store
                .put(C::KEY, ctx.owner_id(), session_id, state)
                .await
                .map_err(ExecutorError::PendingStore)?;
            Ok(ResolveResponse::Pending { token, interaction })
        },
        ResolveResult::Retry { after } => Ok(ResolveResponse::Retry { after, token: None }),
    }
}

/// Continue interactive credential resolve with timeout and
/// pending-state handling.
///
/// Bound on [`Interactive`] per Tech Spec — non-interactive
/// credentials cannot reach this dispatch path. The framework loads the
/// typed [`Interactive::Pending`] from the pending store, invokes
/// [`Interactive::continue_resolve`], and persists the next pending
/// state on the multi-step path. The `Pending` variant carries the typed
/// `Self::Pending` produced by [`Interactive::begin`] or the prior continuation.
///
/// The caller-provided [`CredentialContext`] **must** populate
/// `session_id`; a missing session id returns
/// [`ExecutorError::MissingSessionId`] rather than collapsing into a
/// silent shared bucket.
#[tracing::instrument(name = "credential.execute.continue", skip_all, fields(credential_key = C::KEY))]
pub async fn execute_continue<C, S>(
    token: &PendingToken,
    input: &UserInput,
    ctx: &CredentialContext,
    pending_store: &S,
) -> Result<ResolveResponse<C::State>, ExecutorError>
where
    C: Interactive,
    S: PendingStateStore,
{
    let session_id = ctx.session_id().ok_or(ExecutorError::MissingSessionId)?;
    let polling = matches!(input, UserInput::Poll);
    let pending: <C as Interactive>::Pending = if polling {
        pending_store
            .get_bound(C::KEY, token, ctx.owner_id(), session_id)
            .await
    } else {
        pending_store
            .consume(C::KEY, token, ctx.owner_id(), session_id)
            .await
    }
    .map_err(ExecutorError::PendingStore)?;

    let result = tokio::time::timeout(
        CREDENTIAL_TIMEOUT,
        <C as Interactive>::continue_resolve(&pending, input, ctx),
    )
    .await
    .map_err(|_| {
        if polling {
            ExecutorError::Timeout {
                timeout: CREDENTIAL_TIMEOUT,
            }
        } else {
            ExecutorError::Credential(CredentialError::OutcomeUnknown)
        }
    })?
    .map_err(ExecutorError::Credential)?;

    match result {
        ResolveResult::Complete(state) => {
            if polling {
                let _consumed: <C as Interactive>::Pending = pending_store
                    .consume(C::KEY, token, ctx.owner_id(), session_id)
                    .await
                    .map_err(ExecutorError::PendingStore)?;
            }
            Ok(ResolveResponse::Complete(state))
        },
        ResolveResult::Pending { state, interaction } => {
            let next_token = pending_store
                .put(C::KEY, ctx.owner_id(), session_id, state)
                .await
                .map_err(ExecutorError::PendingStore)?;

            if polling
                && let Err(err) = pending_store
                    .consume::<<C as Interactive>::Pending>(
                        C::KEY,
                        token,
                        ctx.owner_id(),
                        session_id,
                    )
                    .await
            {
                // Best-effort cleanup: the new state was stored but the caller
                // will never receive `next_token` because we are returning an
                // error. Delete it to avoid a permanent store leak.
                // The delete error (if any) is subordinate to the primary error.
                let _ = pending_store.delete(&next_token).await;
                return Err(ExecutorError::PendingStore(err));
            }

            Ok(ResolveResponse::Pending {
                token: next_token,
                interaction,
            })
        },
        ResolveResult::Retry { after } if polling => Ok(ResolveResponse::Retry {
            after,
            token: Some(token.clone()),
        }),
        ResolveResult::Retry { .. } => {
            tracing::error!("one-shot credential continuation returned Retry");
            Err(ExecutorError::InvalidContinuationOutcome)
        },
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use crate::{
        CredentialContext, OAuth2Credential, OAuth2Pending, PendingState, PendingStateStore,
        PendingStoreError, PendingToken, SecretString, credentials::OAuth2Config,
        resolve::UserInput, scheme::AuthStyle,
    };

    use super::{ExecutorError, execute_continue};

    struct ConsumeOnlyStore {
        pending: Vec<u8>,
    }

    impl PendingStateStore for ConsumeOnlyStore {
        async fn put<P: PendingState>(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: P,
        ) -> Result<PendingToken, PendingStoreError> {
            panic!("continuation must not write pending state")
        }

        async fn get<P: PendingState>(&self, _: &PendingToken) -> Result<P, PendingStoreError> {
            panic!("callback continuation must not read pending state")
        }

        async fn get_bound<P: PendingState>(
            &self,
            _: &str,
            _: &PendingToken,
            _: &str,
            _: &str,
        ) -> Result<P, PendingStoreError> {
            Err(PendingStoreError::ValidationFailed {
                reason: "callback used a non-consuming read".to_owned(),
            })
        }

        async fn consume<P: PendingState>(
            &self,
            _: &str,
            _: &PendingToken,
            _: &str,
            _: &str,
        ) -> Result<P, PendingStoreError> {
            serde_json::from_slice(&self.pending)
                .map_err(|error| PendingStoreError::Backend(Box::new(error)))
        }

        async fn delete(&self, _: &PendingToken) -> Result<(), PendingStoreError> {
            panic!("continuation must not delete pending state")
        }
    }

    #[tokio::test]
    async fn callback_consumes_pending_state_before_credential_dispatch() {
        let pending = OAuth2Pending {
            config: OAuth2Config::authorization_code("https://client.example/callback")
                .auth_url("https://provider.example/authorize")
                .token_url("https://provider.example/token")
                .build(),
            client_id: "client".to_owned(),
            client_secret: SecretString::new("secret"),
            auth_style: AuthStyle::Header,
            pkce_verifier: SecretString::new("verifier"),
            state: "expected-state".to_owned(),
            redirect_uri: "https://client.example/callback".to_owned(),
        };
        let pending =
            crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&pending))
                .expect("serialize pending fixture");
        let store = ConsumeOnlyStore { pending };
        let token = PendingToken::generate();
        let ctx = CredentialContext::for_owner("owner").with_session_id("session");
        let input = UserInput::Callback {
            params: [("state".to_owned(), "wrong-state".to_owned())].into(),
        };

        let result = execute_continue::<OAuth2Credential, _>(&token, &input, &ctx, &store).await;

        assert_matches!(
            result,
            Err(ExecutorError::Credential(
                crate::CredentialError::InvalidInput
            ))
        );
    }
}
