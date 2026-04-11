//! Framework executor for credential resolution.
//!
//! Wraps [`Credential::resolve()`] and [`Credential::continue_resolve()`]
//! with 30s timeouts and [`PendingStateStore`] lifecycle management.
//! Credential authors write pure functions; the executor handles storage.

use std::time::Duration;

use nebula_parameter::values::ParameterValues;

use crate::{
    context::CredentialContext,
    credential::Credential,
    error::CredentialError,
    pending::PendingToken,
    pending_store::{PendingStateStore, PendingStoreError},
    resolve::{InteractionRequest, ResolveResult, UserInput},
};

/// Default timeout for credential operations.
const CREDENTIAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Outcome of framework-managed credential resolution.
///
/// Returned by [`execute_resolve`] and [`execute_continue`]. The framework
/// matches on this to decide whether to store the credential, present a
/// UI interaction, or schedule a retry poll.
#[derive(Debug)]
pub enum ResolveResponse<S> {
    /// Credential ready -- state should be encrypted and stored.
    Complete(S),

    /// User interaction required -- `PendingState` stored, token generated.
    Pending {
        /// Opaque token for the stored pending state.
        token: PendingToken,
        /// What the UI should show or redirect.
        interaction: InteractionRequest,
    },

    /// Framework should poll `continue_resolve()` after a delay.
    Retry {
        /// How long to wait before polling.
        after: Duration,
    },
}

/// Error during framework-managed resolution.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExecutorError {
    /// Credential method timed out (30s hard limit).
    #[error("credential operation timed out after {timeout:?}")]
    Timeout {
        /// The timeout duration that was exceeded.
        timeout: Duration,
    },

    /// Error from the credential implementation.
    #[error("credential error: {0}")]
    Credential(#[from] CredentialError),

    /// Error storing or loading pending state.
    #[error("pending store error: {0}")]
    PendingStore(#[from] PendingStoreError),
}

/// Execute initial credential resolution with timeout and `PendingState` management.
///
/// Calls [`Credential::resolve`] with a 30s timeout and manages the
/// `PendingState` lifecycle:
///
/// - **`Complete`** -- returns state for the caller to encrypt and store.
/// - **`Pending`** -- stores `PendingState` via `pending_store`, returns a [`PendingToken`] and
///   [`InteractionRequest`] for the UI.
/// - **`Retry`** -- returns the retry delay for the framework to schedule a subsequent
///   `continue_resolve()` poll.
///
/// # Errors
///
/// - [`ExecutorError::Timeout`] if the credential method exceeds 30s.
/// - [`ExecutorError::Credential`] if the credential returns an error.
/// - [`ExecutorError::PendingStore`] if pending state storage fails.
pub async fn execute_resolve<C, S>(
    values: &ParameterValues,
    ctx: &CredentialContext,
    pending_store: &S,
) -> Result<ResolveResponse<C::State>, ExecutorError>
where
    C: Credential,
    S: PendingStateStore,
{
    let result = tokio::time::timeout(CREDENTIAL_TIMEOUT, C::resolve(values, ctx))
        .await
        .map_err(|_| ExecutorError::Timeout {
            timeout: CREDENTIAL_TIMEOUT,
        })?
        .map_err(ExecutorError::Credential)?;

    handle_resolve_result::<C, S>(result, ctx, pending_store).await
}

/// Continue interactive resolution after the user completes an interaction.
///
/// Loads and consumes `PendingState` from `pending_store` (single-use),
/// then calls [`Credential::continue_resolve`] with a 30s timeout.
/// The result is handled identically to [`execute_resolve`].
///
/// # Errors
///
/// - [`ExecutorError::PendingStore`] if pending state loading/validation fails.
/// - [`ExecutorError::Timeout`] if the credential method exceeds 30s.
/// - [`ExecutorError::Credential`] if the credential returns an error.
pub async fn execute_continue<C, S>(
    token: &PendingToken,
    input: &UserInput,
    ctx: &CredentialContext,
    pending_store: &S,
) -> Result<ResolveResponse<C::State>, ExecutorError>
where
    C: Credential,
    S: PendingStateStore,
{
    let session_id = ctx.session_id().unwrap_or("default");
    let pending: C::Pending = pending_store
        .consume(C::KEY, token, &ctx.owner_id, session_id)
        .await
        .map_err(ExecutorError::PendingStore)?;

    let result = tokio::time::timeout(
        CREDENTIAL_TIMEOUT,
        C::continue_resolve(&pending, input, ctx),
    )
    .await
    .map_err(|_| ExecutorError::Timeout {
        timeout: CREDENTIAL_TIMEOUT,
    })?
    .map_err(ExecutorError::Credential)?;

    handle_resolve_result::<C, S>(result, ctx, pending_store).await
}

/// Shared handler for `ResolveResult` -- stores pending state or passes
/// through complete/retry variants.
async fn handle_resolve_result<C, S>(
    result: ResolveResult<C::State, C::Pending>,
    ctx: &CredentialContext,
    pending_store: &S,
) -> Result<ResolveResponse<C::State>, ExecutorError>
where
    C: Credential,
    S: PendingStateStore,
{
    match result {
        ResolveResult::Complete(state) => Ok(ResolveResponse::Complete(state)),
        ResolveResult::Pending { state, interaction } => {
            let session_id = ctx.session_id().unwrap_or("default");
            let token = pending_store
                .put(C::KEY, &ctx.owner_id, session_id, state)
                .await
                .map_err(ExecutorError::PendingStore)?;
            Ok(ResolveResponse::Pending { token, interaction })
        }
        ResolveResult::Retry { after } => Ok(ResolveResponse::Retry { after }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{credentials::ApiKeyCredential, pending_store_memory::InMemoryPendingStore};

    #[tokio::test]
    async fn execute_resolve_static_credential_returns_complete() {
        let store = InMemoryPendingStore::new();
        let ctx = CredentialContext::new("user-1");

        let mut values = ParameterValues::new();
        values.set(
            "api_key".to_owned(),
            serde_json::Value::String("sk-test-key".into()),
        );

        let result = execute_resolve::<ApiKeyCredential, _>(&values, &ctx, &store).await;

        assert!(
            matches!(result, Ok(ResolveResponse::Complete(_))),
            "expected Complete, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn execute_resolve_propagates_credential_error() {
        let store = InMemoryPendingStore::new();
        let ctx = CredentialContext::new("user-1");
        let values = ParameterValues::new(); // missing required api_key

        let result = execute_resolve::<ApiKeyCredential, _>(&values, &ctx, &store).await;

        assert!(
            matches!(result, Err(ExecutorError::Credential(_))),
            "expected Credential error, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn execute_continue_returns_pending_store_error_for_missing_token() {
        let store = InMemoryPendingStore::new();
        let ctx = CredentialContext::new("user-1").with_session_id("sess-1");
        let bogus_token = PendingToken::generate();
        let input = UserInput::Poll;

        let result =
            execute_continue::<ApiKeyCredential, _>(&bogus_token, &input, &ctx, &store).await;

        assert!(
            matches!(
                result,
                Err(ExecutorError::PendingStore(PendingStoreError::NotFound))
            ),
            "expected PendingStore NotFound error, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn executor_error_display() {
        let timeout_err = ExecutorError::Timeout {
            timeout: Duration::from_secs(30),
        };
        assert!(timeout_err.to_string().contains("timed out"));

        let cred_err = ExecutorError::Credential(CredentialError::NotInteractive);
        assert!(cred_err.to_string().contains("credential error"));

        let store_err = ExecutorError::PendingStore(PendingStoreError::NotFound);
        assert!(store_err.to_string().contains("pending store"));
    }
}
