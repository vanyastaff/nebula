//! Object-safe supertrait for credential dependency declaration.

use std::any::Any;

use crate::core::CredentialDescription;

/// Object-safe supertrait for declaring credential dependencies.
///
/// `Resource` and `Action` return `Box<dyn AnyCredential>` to declare
/// "I need a credential of this type." The engine uses `Any::type_id()` on
/// `dyn AnyCredential` to identify the credential type at registration time.
///
/// Automatically implemented for all `C: Credential` via the blanket impl below.
pub trait AnyCredential: Any + Send + Sync + 'static {
    /// The normalized key identifying this credential type.
    fn credential_key(&self) -> &str;
    /// Human-readable description of this credential type.
    fn description(&self) -> CredentialDescription;
}

/// Blanket impl: every `Credential` is automatically an `AnyCredential`.
impl<C: crate::Credential + 'static> AnyCredential for C {
    fn credential_key(&self) -> &str {
        // SAFETY: Credential::KEY is a static string reference -- always valid.
        C::KEY
    }

    fn description(&self) -> CredentialDescription {
        C::description()
    }
}
