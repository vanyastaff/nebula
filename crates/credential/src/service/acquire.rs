//! Acquisition surface of [`CredentialService`] — the `resolve` /
//! `continue_resolve` interactive-acquisition flow.
//!
//! Split out of `facade.rs` (behaviour-preserving code motion — no logic
//! change) so the CRUD facade stays focused. Kept in the `service` module
//! so it reads the same `pub(crate)` [`CredentialService`] internals
//! (`ensure_local_source`, `owner_context`, `persist_resolved`).

use serde_json::Value;

use crate::resolve::UserInput;
use crate::{CredentialDisplay, CredentialId, PendingToken};

use super::error::CredentialServiceError;
use super::facade::{Acquisition, CredentialService};
use super::scope::TenantScope;

impl CredentialService {
    /// Acquire a credential of `credential_key` from `props`, persisting
    /// it on synchronous completion or surfacing an interaction token for
    /// interactive flows.
    ///
    /// Validation is the canonical credential pipeline (the `$expr`
    /// refusal point, credential secrecy). A `Complete` resolution is persisted
    /// through the same path as [`create`](Self::create) and returned as
    /// [`Acquisition::Complete`]; a `Pending` kickoff returns
    /// [`Acquisition::Pending`] with the opaque token + UI instruction.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::TypeUnknown`] — key not registered.
    /// - [`CredentialServiceError::ValidationFailed`] — schema / typed-deserialize / resolve.
    /// - [`CredentialServiceError::SessionRequired`] — the resolution
    ///   went `Pending` (interactive kickoff) but `scope` carries no
    ///   session, so the issued token could never be redeemed.
    /// - [`CredentialServiceError::Store`] — persistence failure on the `Complete` path.
    pub async fn resolve(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        props: Value,
    ) -> Result<Acquisition, CredentialServiceError> {
        self.ensure_local_source()?;
        if !self.registry.contains(credential_key) {
            return Err(CredentialServiceError::TypeUnknown {
                key: credential_key.to_owned(),
            });
        }
        self.ops.validate(credential_key, &props)?;
        let values = self.ops.ingest(credential_key, &props)?;
        let ctx = Self::owner_context(scope);
        let outcome = self
            .ops
            .acquire(credential_key, &values, &ctx, &self.pending)
            .await?;
        self.finish_acquire(scope, credential_key, outcome).await
    }

    /// Continue an interactive acquisition with the user's input.
    ///
    /// Threads the service's pending store through the engine's
    /// `execute_continue` for the concrete interactive type. The three
    /// first-party builtins are non-interactive, so no continuation
    /// closure is registered for them and this returns
    /// [`CapabilityUnsupported`](CredentialServiceError::CapabilityUnsupported)
    /// (or [`TypeUnknown`](CredentialServiceError::TypeUnknown) for an
    /// unregistered key).
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::TypeUnknown`] — key not registered.
    /// - [`CredentialServiceError::SessionRequired`] — `scope` carries no
    ///   session; the pending-store binding makes a continuation
    ///   structurally impossible without one.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Interactive`.
    /// - [`CredentialServiceError::ValidationFailed`] — continuation failed.
    /// - [`CredentialServiceError::Store`] — persistence failure on the `Complete` path.
    pub async fn continue_resolve(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        pending_token: &str,
        user_input: UserInput,
    ) -> Result<Acquisition, CredentialServiceError> {
        self.ensure_local_source()?;
        if !self.registry.contains(credential_key) {
            return Err(CredentialServiceError::TypeUnknown {
                key: credential_key.to_owned(),
            });
        }
        // A continuation is structurally dead without a session: the
        // engine's `execute_continue` requires `ctx.session_id()` and the
        // `PendingStateStore` binds the pending on
        // `(kind, owner, session, token)`. Surface that explicitly here
        // rather than letting it collapse into a misleading
        // `ValidationFailed` deep inside the executor.
        if scope.session_id().is_none() {
            return Err(CredentialServiceError::SessionRequired {
                capability: "continue",
            });
        }
        // `PendingToken` has no public string constructor; its
        // documented wire form is a bare JSON string (see its
        // serde round-trip contract), so reconstruct the client-returned
        // token through serde — the only public inbound path.
        let token: PendingToken = serde_json::from_value(Value::String(pending_token.to_owned()))
            .map_err(|_| CredentialServiceError::ValidationFailed {
            reason: "malformed pending acquisition token".to_owned(),
        })?;
        let ctx = Self::owner_context(scope);
        let outcome = self
            .ops
            .continue_resolve(credential_key, &token, &user_input, &ctx, &self.pending)
            .await?;
        self.finish_acquire(scope, credential_key, outcome).await
    }

    /// Map an [`AcquireOutcome`](super::ops::AcquireOutcome) into the public [`Acquisition`]:
    /// `Complete` is persisted (shared create path); `Pending`/`Retry`
    /// surface the token + interaction without persisting.
    async fn finish_acquire(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        outcome: super::ops::AcquireOutcome,
    ) -> Result<Acquisition, CredentialServiceError> {
        match outcome {
            super::ops::AcquireOutcome::Complete(resolved) => {
                let id = CredentialId::new();
                // Acquisition carries no caller-supplied display metadata
                // (the interactive/resolve flow names nothing); a later
                // `update` can attach it.
                let head = self
                    .persist_resolved(
                        scope,
                        credential_key,
                        id,
                        resolved,
                        CredentialDisplay::default(),
                    )
                    .await?;
                self.observer.on_resolve(&id);
                tracing::info!(
                    credential.key = credential_key,
                    credential.id = %id,
                    "credential acquired"
                );
                Ok(Acquisition::Complete { head })
            },
            super::ops::AcquireOutcome::Pending { token, interaction } => {
                // The interaction can only be completed through
                // `continue_resolve`, which the engine binds on
                // `(kind, owner, session, token)`. Without a session on
                // the scope the issued token is unusable, so refuse the
                // kickoff explicitly instead of handing back a token that
                // can never be redeemed.
                if scope.session_id().is_none() {
                    return Err(CredentialServiceError::SessionRequired {
                        capability: "resolve",
                    });
                }
                Ok(Acquisition::Pending {
                    token: token.as_str().to_owned(),
                    interaction,
                })
            },
            super::ops::AcquireOutcome::Retry { after } => Ok(Acquisition::Retry { after }),
        }
    }
}
