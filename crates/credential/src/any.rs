//! Object-safe supertrait for credential dependency declaration.
use std::any::Any;

use crate::core::CredentialDescription;
use crate::traits::CredentialType;

/// Object-safe supertrait for declaring credential dependencies.
///
/// `Resource` and `Action` return `Box<dyn AnyCredential>` to declare
/// "I need a credential of this type." The engine uses `Any::type_id()` on
/// `dyn AnyCredential` to identify the credential type at registration time.
///
/// Automatically implemented for all `C: CredentialType` via the blanket impl below.
pub trait AnyCredential: Any + Send + Sync + 'static {
    /// The normalized key identifying this credential type.
    fn credential_key(&self) -> &str;
    /// Human-readable description of this credential type.
    fn description(&self) -> CredentialDescription;
}

/// Blanket impl: every `CredentialType` is automatically an `AnyCredential`.
impl<C: CredentialType + 'static> AnyCredential for C {
    fn credential_key(&self) -> &str {
        // SAFETY: `CredentialKey` is a `Deref<Target=str>` backed by an interned
        // static or leaked string. Leaking here is acceptable because credential
        // keys are registered once at startup and live for the program lifetime.
        Box::leak(C::credential_key().to_string().into_boxed_str())
    }

    fn description(&self) -> CredentialDescription {
        C::description()
    }
}
