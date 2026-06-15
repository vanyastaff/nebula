//! Typed credential reference (`CredentialRef<C>`) — slot-binding handle for action/resource
//! fields.
//!
//! Per typed ref fields, `CredentialRef<C>` is the canonical field type for the
//! slot-binding pattern: an action or resource declares
//!
//! ```ignore
//! #[derive(Action)]
//! struct SendTelegram {
//!     #[credential(key = "auth")]
//!     token: CredentialRef<TelegramCredential>,   // lazy variant — see CredentialGuard for eager
//! }
//! ```
//!
//! and the framework resolves the slot per slot binding's binding mechanism: the
//! `key` attribute supplies a default credential id; workflow-JSON
//! `slot_bindings.<slot_key>.credential_id` overrides it per node.
//!
//! ## Phase 1 scope (current)
//!
//! `CredentialRef<C>` carries the resolved credential id and a `PhantomData<C>`
//! marker. `.resolve(ctx)` delegates to the existing `HasCredentials::resolve_any`
//! path (the dyn-erased accessor) and downcasts via the credential snapshot
//! pipeline. Phase 6 of the M6 resource-finalization integration work adds
//! engine-side typed helpers (`ctx.resolve_credential_by_id::<C>(id)`); this
//! type's `.resolve` will switch over without API change.
//!
//! ## Composability
//!
//! `Option<CredentialRef<C>>` and `Lazy<CredentialRef<C>>` (from
//! `nebula_core::Lazy`) compose for optional / lazy semantics per optional ref composition.

use std::{fmt, marker::PhantomData};

use nebula_core::{CredentialKey, context::capability::HasCredentials};
use zeroize::Zeroize;

use crate::{Credential, CredentialGuard, error::CredentialError, snapshot::CredentialSnapshot};

/// Typed reference to a registered credential.
///
/// The reference carries a credential id (slot binding per slot binding) and a
/// type-level marker selecting the concrete `Credential` impl whose
/// `Scheme` should be projected on resolve. The field type alone tells
/// the framework what slot kind, what concrete credential type, and (via
/// wrapper composition) whether resolution is eager / lazy / optional.
///
/// Use [`CredentialRef::resolve`] inside action / resource bodies to obtain
/// a zeroizing [`CredentialGuard<C::Scheme>`] holding the projected auth material.
///
/// `CredentialRef<C>` is `Clone` and cheap (id + zero-sized phantom). Cloning
/// does **not** copy the underlying secret — the secret materializes only
/// during `.resolve()` and lives in the resulting guard with zeroize-on-drop.
pub struct CredentialRef<C: ?Sized> {
    /// Resolved credential id (per slot binding — slot-key default OR workflow-JSON override).
    id: String,
    /// Type-level marker for the concrete `Credential` impl. `fn() -> C` so
    /// `CredentialRef<C>` is `Send + Sync` regardless of `C`.
    _phantom: PhantomData<fn() -> C>,
}

impl<C: ?Sized> CredentialRef<C> {
    /// Constructs a reference bound to the given credential id.
    ///
    /// Typically called by the macro-emitted [`crate::Credential`]-derive /
    /// `#[derive(Action)]` factory body — plugin authors do not write this
    /// directly. The `id` is sourced from the slot's `key` attribute or from
    /// the workflow node's `slot_bindings` override.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            _phantom: PhantomData,
        }
    }

    /// Returns the resolved credential id this reference binds to.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl<C: Credential> CredentialRef<C> {
    /// Resolves the reference to a [`CredentialGuard<C::Scheme>`].
    ///
    /// Builds a [`CredentialKey`] from the carried id, calls `resolve_any`
    /// on the context's credential accessor, downcasts the result to
    /// [`CredentialSnapshot`], projects to `C::Scheme`, and wraps in a
    /// zeroizing guard.
    ///
    /// # Errors
    ///
    /// - [`CredentialError::InvalidInput`] — id cannot be coerced into a valid [`CredentialKey`],
    ///   or the accessor returned an unexpected type.
    /// - [`CredentialError::Resolution`] — the credential is not registered under the id.
    /// - [`CredentialError::SchemeMismatch`] — the credential resolves but carries a different
    ///   `AuthScheme` than `C::Scheme`.
    pub async fn resolve<Ctx>(
        &self,
        ctx: &Ctx,
    ) -> Result<CredentialGuard<C::Scheme>, CredentialError>
    where
        Ctx: HasCredentials + Sync + ?Sized,
        C::Scheme: Zeroize,
    {
        let key = CredentialKey::new(&self.id).map_err(|e| {
            CredentialError::InvalidInput(format!(
                "credential id `{id}` is not a valid CredentialKey: {e}",
                id = self.id
            ))
        })?;

        let boxed = ctx
            .credentials()
            .resolve_any(&key)
            .await
            .map_err(CredentialError::from)?;

        let snapshot = boxed.downcast::<CredentialSnapshot>().map_err(|_| {
            CredentialError::InvalidInput(format!(
                "credential `{id}`: resolve_any returned unexpected type \
                 (expected CredentialSnapshot)",
                id = self.id
            ))
        })?;

        let scheme = snapshot.into_project::<C::Scheme>().map_err(|e| match e {
            crate::snapshot::SnapshotError::SchemeMismatch { expected, actual } => {
                CredentialError::SchemeMismatch(Box::new(crate::error::SchemeMismatch::by_name(
                    expected, actual,
                )))
            },
        })?;

        Ok(CredentialGuard::new(scheme))
    }
}

impl<C: ?Sized> Clone for CredentialRef<C> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<C: ?Sized> fmt::Debug for CredentialRef<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialRef")
            .field("id", &self.id)
            .field("type", &std::any::type_name::<C>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Phantom credential type for tests — the resolve() path is exercised in
    // integration tests once Phase 6 wires the engine-side helpers; these
    // unit tests cover construction + accessors + Debug + Clone.
    struct FakeCred;

    #[test]
    fn new_carries_id() {
        let r: CredentialRef<FakeCred> = CredentialRef::new("acme-bot");
        assert_eq!(r.id(), "acme-bot");
    }

    #[test]
    fn clone_preserves_id() {
        let r: CredentialRef<FakeCred> = CredentialRef::new("primary");
        let cloned = r.clone();
        assert_eq!(r.id(), cloned.id());
    }

    #[test]
    fn debug_includes_id_and_type() {
        let r: CredentialRef<FakeCred> = CredentialRef::new("cred-1");
        let s = format!("{r:?}");
        assert!(s.contains("cred-1"));
        assert!(s.contains("FakeCred"));
    }
}
