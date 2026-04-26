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
    /// Base [`Credential::resolve`] returned [`ResolveResult::Pending`] — but
    /// `state: ()` is degenerate and cannot deserialize into the typed
    /// [`Interactive::Pending`] used by [`execute_continue`].
    ///
    /// Per Tech Spec §15.4 the base [`Credential::resolve`] **must** return
    /// [`ResolveResult::Complete`] (or `Retry`); interactive kick-offs that
    /// need typed pending state populate it directly via
    /// [`PendingStateStore::put`] from a credential-specific helper such as
    /// [`OAuth2Credential::initiate_authorization_code`]. Routing a base
    /// `Pending(())` through this executor would persist `()` bytes and
    /// then fail to deserialize on `execute_continue::<C: Interactive, _>`
    /// for any credential whose `Pending` is not unit. Fail at kickoff
    /// instead of at continuation — the diagnostic surfaces at the right
    /// site.
    ///
    /// [`PendingStateStore::put`]: nebula_credential::pending_store::PendingStateStore::put
    /// [`OAuth2Credential::initiate_authorization_code`]: nebula_credential::credentials::OAuth2Credential::initiate_authorization_code
    #[error(
        "base Credential::resolve returned Pending; interactive flows must use \
         credential-specific kickoff helpers + PendingStateStore::put directly \
         (per Tech Spec §15.4)"
    )]
    BaseResolvePending,
}

/// Execute initial credential resolve with timeout and pending-state
/// handling.
///
/// Per Tech Spec §15.4 the base [`Credential::resolve`] returns
/// `ResolveResult<State, ()>`. The contract for this executor is:
///
/// - **`Complete(state)`** — credential resolved synchronously; caller persists the state.
/// - **`Retry { after }`** — credential is in-flight; caller polls again after the delay.
/// - **`Pending { state: () }`** — **rejected** with [`ExecutorError::BaseResolvePending`].
///   Routing `()` through `PendingStateStore::put` and later trying to
///   `get_bound::<C::Pending>` on `execute_continue` would fail to
///   deserialize for any non-unit pending. Interactive credentials use
///   credential-specific kickoff helpers (e.g.
///   [`OAuth2Credential::initiate_authorization_code`](nebula_credential::credentials::OAuth2Credential::initiate_authorization_code))
///   that construct their typed `Self::Pending` directly and persist it
///   via [`PendingStateStore::put`] — this executor is **not** the
///   kickoff path for those flows.
///
/// Bound on [`Credential`] (no `Interactive` requirement) so this works
/// for non-interactive credentials too; interactive continuation uses
/// [`execute_continue`].
///
/// [`PendingStateStore::put`]: nebula_credential::pending_store::PendingStateStore::put
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
        ResolveResult::Complete(state) => {
            // Drain the unused store handle so unused-bind checks stay quiet
            // when no pending path fires.
            let _ = pending_store;
            Ok(ResolveResponse::Complete(state))
        },
        ResolveResult::Pending { .. } => {
            // §15.4: base resolve cannot return Pending — the typed Pending
            // path lives on Interactive. See `ExecutorError::BaseResolvePending`
            // doc-comment for the rationale.
            let _ = pending_store;
            Err(ExecutorError::BaseResolvePending)
        },
        ResolveResult::Retry { after } => {
            let _ = pending_store;
            Ok(ResolveResponse::Retry { after, token: None })
        },
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
/// carries the typed `Self::Pending` populated by the credential's
/// kickoff helper (not by [`execute_resolve`], which rejects base
/// `Pending` per the contract above).
///
/// The caller-provided [`CredentialContext`] **must** populate
/// `session_id`; a missing session id returns
/// [`ExecutorError::MissingSessionId`] rather than collapsing into a
/// silent shared bucket.
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
