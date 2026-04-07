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
/// use nebula_core::AuthScheme;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Clone, Serialize, Deserialize)]
/// struct BearerToken {
///     token: String,
/// }
///
/// impl AuthScheme for BearerToken {
///     const KIND: &'static str = "bearer";
/// }
/// ```
pub trait AuthScheme: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {
    /// Unique identifier for this scheme type (e.g., `"bearer"`, `"basic"`).
    const KIND: &'static str;

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
    const KIND: &'static str = "none";
}
