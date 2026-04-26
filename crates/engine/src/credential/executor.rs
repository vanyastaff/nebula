//! Framework executor for credential resolution.

use std::time::Duration;

use nebula_credential::{
    Credential, CredentialContext, Interactive, PendingToken,
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
}

/// Execute initial credential resolve with timeout and pending-state
/// handling.
///
/// Per Tech Spec §15.4 the base [`Credential::resolve`] returns
/// `ResolveResult<State, ()>` — typed pending state moves to
/// [`Interactive::continue_resolve`]. For non-interactive credentials
/// this entry point handles `Complete` and `Retry` directly. The
/// `Pending` variant carries `state: ()` (degenerate "kickoff" marker)
/// and is bridged through the framework's `PendingStateStore` so
/// downstream `execute_continue::<C: Interactive, _>` can begin the
/// typed-pending continuation. Interactive credentials whose first
/// step requires PKCE/CSRF state (OAuth2 authorization code, device
/// code) construct their typed `Self::Pending` directly via
/// credential-specific kickoff helpers (see
/// [`OAuth2Credential::initiate_authorization_code`](nebula_credential::credentials::OAuth2Credential::initiate_authorization_code))
/// and persist it via
/// [`PendingStateStore::put`](nebula_credential::pending_store::PendingStateStore::put)
/// — this executor is *not* the kickoff path for those flows.
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

    match result {
        ResolveResult::Complete(state) => Ok(ResolveResponse::Complete(state)),
        ResolveResult::Pending { state, interaction } => {
            // Per §15.4 the base trait carries `state: ()` here — a
            // degenerate pending value. Storing it gives the caller a
            // session-bound token that can be used to thread through
            // `execute_continue::<C: Interactive, _>` once the
            // credential-specific kickoff helper has populated the
            // typed `Self::Pending` for that token.
            let session_id = ctx.session_id().unwrap_or("default");
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
/// Bound on [`Interactive`] per Tech Spec §15.4 — non-interactive
/// credentials cannot reach this dispatch path. The framework loads the
/// typed [`Interactive::Pending`] from the pending store, invokes
/// [`Interactive::continue_resolve`], and persists the next pending
/// state on the multi-step path. The `Pending` variant of the result
/// carries the typed `Self::Pending`, distinct from the degenerate
/// `()` kickoff in [`execute_resolve`].
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
    let session_id = ctx.session_id().unwrap_or("default");
    let pending: <C as Interactive>::Pending = pending_store
        .get_bound(C::KEY, token, ctx.owner_id(), session_id)
        .await
        .map_err(ExecutorError::PendingStore)?;

    let result = tokio::time::timeout(
        CREDENTIAL_TIMEOUT,
        <C as Interactive>::continue_resolve(&pending, input, ctx),
    )
    .await
    .map_err(|_| ExecutorError::Timeout {
        timeout: CREDENTIAL_TIMEOUT,
    })?
    .map_err(ExecutorError::Credential)?;

    match result {
        ResolveResult::Complete(state) => {
            let _consumed: <C as Interactive>::Pending = pending_store
                .consume(C::KEY, token, ctx.owner_id(), session_id)
                .await
                .map_err(ExecutorError::PendingStore)?;
            Ok(ResolveResponse::Complete(state))
        },
        ResolveResult::Pending { state, interaction } => {
            let next_token = pending_store
                .put(C::KEY, ctx.owner_id(), session_id, state)
                .await
                .map_err(ExecutorError::PendingStore)?;

            if let Err(err) = pending_store
                .consume::<<C as Interactive>::Pending>(C::KEY, token, ctx.owner_id(), session_id)
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
        ResolveResult::Retry { after } => Ok(ResolveResponse::Retry {
            after,
            token: Some(token.clone()),
        }),
    }
}
