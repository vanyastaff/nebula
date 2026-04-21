//! Framework executor for credential resolution.

use std::time::Duration;

use nebula_credential::{
    Credential, CredentialContext, PendingToken,
    error::CredentialError,
    pending_store::{PendingStateStore, PendingStoreError},
    resolve::{InteractionRequest, ResolveResult, UserInput},
};
use nebula_schema::FieldValues;

const CREDENTIAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Outcome of framework-managed credential resolution.
#[derive(Debug)]
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
}

/// Execute initial credential resolve with timeout and pending-state handling.
pub async fn execute_resolve<C, S>(
    values: &FieldValues,
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

/// Continue interactive credential resolve with timeout and pending-state handling.
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
        .get(token)
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

    match result {
        ResolveResult::Complete(state) => {
            let _consumed: C::Pending = pending_store
                .consume(C::KEY, token, &ctx.owner_id, session_id)
                .await
                .map_err(ExecutorError::PendingStore)?;
            Ok(ResolveResponse::Complete(state))
        },
        ResolveResult::Pending { state, interaction } => {
            let _consumed: C::Pending = pending_store
                .consume(C::KEY, token, &ctx.owner_id, session_id)
                .await
                .map_err(ExecutorError::PendingStore)?;

            let next_token = pending_store
                .put(C::KEY, &ctx.owner_id, session_id, state)
                .await
                .map_err(ExecutorError::PendingStore)?;

            Ok(ResolveResponse::Pending {
                token: next_token,
                interaction,
            })
        },
        ResolveResult::Retry { after } => Ok(ResolveResponse::Retry { after }),
    }
}

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
        },
        ResolveResult::Retry { after } => Ok(ResolveResponse::Retry { after }),
    }
}
