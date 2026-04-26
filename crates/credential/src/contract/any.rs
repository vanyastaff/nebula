//! Object-safe supertrait for credential dependency declaration.

use std::any::Any;

use crate::CredentialMetadata;

/// Object-safe supertrait for declaring credential dependencies.
///
/// `Resource` and `Action` return `Box<dyn AnyCredential>` to declare
/// "I need a credential of this type." The engine uses `Any::type_id()`
/// on `dyn AnyCredential` to identify the credential type at
/// registration time.
///
/// Per Tech Spec §15.4 capability sub-trait split — `is_dynamic()` and
/// `lease_ttl()` (which read the removed `C::DYNAMIC` / `C::LEASE_TTL`
/// const-bool capability flags) have been dropped. Capability discovery
/// over `dyn AnyCredential` moves to the
/// [`CredentialRegistry`](crate::CredentialRegistry) capability set
/// computed at registration time from sub-trait membership (§15.8 /
/// Stage 7).
///
/// Automatically implemented for all `C: Credential` via the blanket
/// impl below.
pub trait AnyCredential: Any + Send + Sync + 'static {
    /// The normalized key identifying this credential type.
    fn credential_key(&self) -> &str;
    /// Integration-catalog metadata describing this credential type.
    fn metadata(&self) -> CredentialMetadata;
}

/// Blanket impl: every `Credential` is automatically an `AnyCredential`.
impl<C: crate::Credential + 'static> AnyCredential for C {
    fn credential_key(&self) -> &str {
        // SAFETY: Credential::KEY is a static string reference -- always valid.
        C::KEY
    }

    fn metadata(&self) -> CredentialMetadata {
        C::metadata()
    }
}
