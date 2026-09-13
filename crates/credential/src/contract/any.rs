//! Object-safe credential contract used by plugin registration.

use std::any::Any;

use super::capability_report::{
    Capabilities, compute_capabilities,
    plugin_capability_report::{IsDynamic, IsInteractive, IsRefreshable, IsRevocable, IsTestable},
};
use crate::CredentialMetadata;

mod sealed {
    use super::{IsDynamic, IsInteractive, IsRefreshable, IsRevocable, IsTestable};

    pub trait Sealed {}

    impl<C> Sealed for C where
        C: crate::Credential
            + IsInteractive
            + IsRefreshable
            + IsRevocable
            + IsTestable
            + IsDynamic
            + 'static
    {
    }
}

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
/// This trait is sealed and automatically implemented for typed
/// [`crate::Credential`] implementations that supply all five capability
/// reports. Integrations cannot self-attest erased keys, metadata,
/// capabilities, or downcast identity.
pub trait AnyCredential: sealed::Sealed + Any + Send + Sync + 'static {
    /// The normalized key identifying this credential type.
    fn credential_key(&self) -> &str;
    /// Integration-catalog metadata describing this credential type.
    fn metadata(&self) -> Result<CredentialMetadata, crate::CredentialMetadataAdmissionError>;
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

    fn metadata(&self) -> Result<CredentialMetadata, crate::CredentialMetadataAdmissionError> {
        C::metadata().admit_for::<C>()
    }

    fn capabilities(&self) -> Capabilities {
        compute_capabilities::<C>()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
