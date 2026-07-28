//! Object-safe credential contract used by plugin registration.

use std::any::Any;

use super::capability_report::{
    Capabilities, compute_capabilities,
    plugin_capability_report::{IsDynamic, IsInteractive, IsRefreshable, IsRevocable, IsTestable},
};
use crate::CredentialMetadata;

/// Object-safe credential projection for plugin registration and discovery.
///
/// Plugin implementations contribute credentials as `Arc<dyn AnyCredential>`
/// values. Resource and action dependency contracts carry typed key and
/// [`std::any::TypeId`] facts that activation can compare with this erased
/// registry projection.
///
/// The blanket implementation projects [`Capabilities`] with
/// [`compute_capabilities`], so erased discovery observes the same five
/// report traits as [`crate::CredentialRegistry`]. [`Capabilities::empty`]
/// is therefore exact for a credential whose five reports are all false.
/// A direct implementation is a trusted escape hatch and must return the
/// concrete type's exact declared report surface; a fabricated default-empty
/// result for a capable type would violate plugin activation coherence.
/// Direct implementations must also return `self` from [`Self::as_any`];
/// plugin resolution compares that downcast projection with the inherited
/// [`Any::type_id`] and rejects an incoherent implementation.
///
/// Automatically implemented for all `C: Credential` via the blanket
/// impl below when the credential supplies all five capability reports.
pub trait AnyCredential: Any + Send + Sync + 'static {
    /// The normalized key identifying this credential type.
    fn credential_key(&self) -> &str;
    /// Integration-catalog metadata describing this credential type.
    fn metadata(&self) -> CredentialMetadata;
    /// Capabilities computed from the credential's five report traits.
    fn capabilities(&self) -> Capabilities;
    /// Type-erased `self` for downcast — required by the KEY-keyed
    /// `CredentialRegistry` (Tech Spec §3.1) to return concrete
    /// `&C` references via `Any::downcast_ref` after registry lookup.
    ///
    /// Implementations must return `self`, not another `Any` value.
    fn as_any(&self) -> &dyn Any;
}

/// Blanket impl: every `Credential` is automatically an `AnyCredential`.
impl<C> AnyCredential for C
where
    C: crate::Credential
        + IsInteractive
        + IsRefreshable
        + IsRevocable
        + IsTestable
        + IsDynamic
        + 'static,
{
    fn credential_key(&self) -> &str {
        // SAFETY: Credential::KEY is a static string reference -- always valid.
        C::KEY
    }

    fn metadata(&self) -> CredentialMetadata {
        C::metadata()
    }

    fn capabilities(&self) -> Capabilities {
        compute_capabilities::<C>()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
