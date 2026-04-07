//! Authentication scheme contract types.
//!
//! [`AuthScheme`] is the bridge between the credential system and the
//! resource system. Resources declare what auth material they need
//! (`type Auth: AuthScheme`), and credentials produce it via `project()`.
//!
//! This trait lives in nebula-core because both nebula-credential and
//! nebula-resource depend on it — it is a contract type, not an
//! implementation detail of either crate.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::AuthPattern;

/// Consumer-facing authentication material.
///
/// Resources declare `type Auth: AuthScheme` to specify what auth
/// material they need. Credentials produce it via `Credential::project()`.
///
/// # Security contract
///
/// `Serialize + DeserializeOwned` bounds exist for the State = Scheme
/// identity path (static credentials stored directly). Serialization
/// to plaintext JSON happens **exclusively** inside `EncryptionLayer`.
/// Never serialize `AuthScheme` types in logging, debugging, or telemetry.
///
/// # Implementors
///
/// Built-in schemes are defined in `nebula-credential::scheme`.
/// The `()` type implements `AuthScheme` for resources that require
/// no authentication.
///
/// # Examples
///
/// ```
/// use nebula_core::{AuthScheme, AuthPattern};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Clone, Serialize, Deserialize)]
/// struct BearerToken {
///     token: String,
/// }
///
/// impl AuthScheme for BearerToken {
///     fn pattern() -> AuthPattern {
///         AuthPattern::SecretToken
///     }
/// }
/// ```
pub trait AuthScheme: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {
    /// Classification for UI, logging, and tooling.
    ///
    /// Returns the [`AuthPattern`] that best describes this scheme's
    /// authentication mechanism. Used by framework tooling to categorize
    /// credentials without inspecting their concrete type.
    fn pattern() -> AuthPattern;

    /// When this auth material expires, if applicable.
    ///
    /// Used by the framework to schedule auto-refresh. Returns `None`
    /// for schemes that do not expire (the default).
    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        None
    }
}

/// No authentication required.
impl AuthScheme for () {
    fn pattern() -> AuthPattern {
        AuthPattern::Custom
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthPattern;

    #[derive(Clone, serde::Serialize, serde::Deserialize)]
    struct TestToken {
        value: String,
    }

    impl AuthScheme for TestToken {
        fn pattern() -> AuthPattern {
            AuthPattern::SecretToken
        }
    }

    #[test]
    fn custom_scheme_reports_correct_pattern() {
        assert_eq!(TestToken::pattern(), AuthPattern::SecretToken);
    }

    #[test]
    fn unit_scheme_pattern_is_custom() {
        assert_eq!(<() as AuthScheme>::pattern(), AuthPattern::Custom);
    }
}
