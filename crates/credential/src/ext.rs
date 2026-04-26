//! Typed credential extension trait for capability contexts.
//!
//! [`HasCredentialsExt`] is a blanket extension on any `HasCredentials`
//! (defined in `nebula_core::context::capability`) context, providing
//! ergonomic typed access:
//!
//! ```rust,ignore
//! let guard = ctx.credential::<SlackBotToken>().await?;
//! client.bearer_auth(guard.token.expose_secret());
//! // guard drops → zeroized automatically
//! ```
//!
//! The trait bridges between the dyn-safe `CredentialAccessor` (which
//! returns `Box<dyn Any>`) and the typed `Credential` contract by:
//!
//! 1. Constructing a [`CredentialKey`] from `C::KEY`.
//! 2. Calling `resolve_any` to get a boxed [`CredentialSnapshot`].
//! 3. Downcasting the snapshot and projecting the typed [`AuthScheme`].
//! 4. Wrapping the result in a zeroizing [`CredentialGuard`].

use nebula_core::{CredentialKey, context::capability::HasCredentials};
use zeroize::Zeroize;

use crate::{Credential, CredentialGuard, error::CredentialError, snapshot::CredentialSnapshot};

/// Typed credential access for any context implementing [`HasCredentials`].
///
/// Primary API: `ctx.credential::<SlackBotToken>().await?`
///
/// This is a blanket extension trait — every `HasCredentials` implementor
/// gets these methods for free. The `Sized` bound on each method is
/// intentional: typed credential access requires static dispatch (the
/// `Credential` type is used to select the key and project the scheme).
pub trait HasCredentialsExt: HasCredentials {
    /// Resolve a typed credential and wrap the scheme in a zeroizing guard.
    ///
    /// # Flow
    ///
    /// 1. Builds a [`CredentialKey`] from `C::KEY`.
    /// 2. Calls `resolve_any` on the underlying `CredentialAccessor`.
    /// 3. Downcasts the `Box<dyn Any>` to [`CredentialSnapshot`].
    /// 4. Projects the scheme via [`CredentialSnapshot::into_project`].
    /// 5. Wraps the result in a [`CredentialGuard`] (zeroize on drop).
    ///
    /// # Errors
    ///
    /// - [`CredentialError::Resolution`] if the accessor cannot resolve the key.
    /// - [`CredentialError::InvalidInput`] if the resolved value is not a `CredentialSnapshot`
    ///   (type mismatch in the accessor).
    /// - [`CredentialError::SchemeMismatch`] if the snapshot's scheme type does not match
    ///   `C::Scheme`.
    fn credential<C: Credential>(
        &self,
    ) -> impl Future<Output = Result<CredentialGuard<C::Scheme>, CredentialError>> + Send
    where
        C::Scheme: Zeroize,
        Self: Sized + Sync;

    /// Like [`credential()`](Self::credential) but returns `None` instead of
    /// erroring when the credential is not found or not configured.
    ///
    /// Other errors (scheme mismatch, accessor failures) still propagate.
    fn try_credential<C: Credential>(
        &self,
    ) -> impl Future<Output = Result<Option<CredentialGuard<C::Scheme>>, CredentialError>> + Send
    where
        C::Scheme: Zeroize,
        Self: Sized + Sync;
}

impl<Ctx: HasCredentials + ?Sized> HasCredentialsExt for Ctx {
    async fn credential<C: Credential>(&self) -> Result<CredentialGuard<C::Scheme>, CredentialError>
    where
        C::Scheme: Zeroize,
        Self: Sized + Sync,
    {
        let key = CredentialKey::new(C::KEY)
            .map_err(|e| CredentialError::InvalidInput(format!("invalid credential key: {e}")))?;

        let boxed = self
            .credentials()
            .resolve_any(&key)
            .await
            .map_err(CredentialError::from)?;

        let snapshot = boxed.downcast::<CredentialSnapshot>().map_err(|_| {
            CredentialError::InvalidInput(
                "resolve_any returned unexpected type (expected CredentialSnapshot)".into(),
            )
        })?;

        let scheme = snapshot.into_project::<C::Scheme>().map_err(|e| match e {
            crate::snapshot::SnapshotError::SchemeMismatch { expected, actual } => {
                CredentialError::SchemeMismatch { expected, actual }
            },
        })?;

        Ok(CredentialGuard::new(scheme))
    }

    async fn try_credential<C: Credential>(
        &self,
    ) -> Result<Option<CredentialGuard<C::Scheme>>, CredentialError>
    where
        C::Scheme: Zeroize,
        Self: Sized + Sync,
    {
        match self.credential::<C>().await {
            Ok(guard) => Ok(Some(guard)),
            Err(CredentialError::Resolution { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
