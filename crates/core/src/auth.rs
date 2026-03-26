//! Authentication scheme contract types.
//!
//! [`AuthScheme`] is the bridge between the credential system and the
//! resource system. Resources declare what auth material they need
//! (`type Auth: AuthScheme`), and credentials produce it via `project()`.
//!
//! This trait lives in nebula-core because both nebula-credential and
//! nebula-resource depend on it — it is a contract type, not an
//! implementation detail of either crate.

/// Marker trait for consumer-facing authentication material.
///
/// Resources declare `type Auth: AuthScheme` to specify what auth
/// material they need (e.g., `BearerToken`, `DatabaseAuth`).
/// Credential types produce auth material via `Credential::project()`.
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
///
/// // A simple bearer token scheme
/// #[derive(Clone)]
/// struct BearerToken {
///     token: String,
/// }
///
/// impl AuthScheme for BearerToken {}
/// ```
pub trait AuthScheme: Send + Sync + Clone + 'static {}

/// No authentication required.
impl AuthScheme for () {}
