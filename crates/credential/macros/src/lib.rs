//! Proc-macro crate for the `Credential` derive macro.
//!
//! Generates a Credential impl for static (non-interactive) credentials
//! backed by a StaticProtocol.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod auth_scheme;
mod credential;

/// Derive macro for the v2 `Credential` trait.
///
/// Generates a full Credential impl for static (non-interactive)
/// credentials backed by a StaticProtocol. The generated impl uses
/// `State = Scheme` (identity path), `Pending = NoPendingState`, and
/// all capability flags default to `false`.
///
/// # Attributes
///
/// ## Container attributes (`#[credential(...)]` on the struct)
///
/// - `key = "..."` - Unique credential type key (required)
/// - `name = "..."` - Human-readable name (required)
/// - `scheme = Type` - Auth scheme type, also used as `State` (required)
/// - `protocol = Type` - `StaticProtocol` impl for `parameters()` and `build()` (required)
/// - `icon = "..."` - Icon identifier for UI (optional)
/// - `doc_url = "..."` - Documentation URL (optional)
/// - `dynamic = true` - Whether this credential produces ephemeral per-execution secrets (optional,
///   default `false`)
/// - `lease_ttl_secs = 300` - Lease duration in seconds for dynamic credentials (optional, default
///   `None`)
///
/// ## Dependency attributes (outer attributes on the struct)
///
/// - `#[uses_resource(TypeName, purpose = "...")]` - Declare a resource dependency (repeatable)
/// - `#[uses_credential(...)]` - **Forbidden** — emits a compile error (spec 23)
///
/// # Example
///
/// ```ignore
/// use nebula_credential::{Credential, StaticProtocol};
///
/// #[derive(Credential)]
/// #[credential(
///     key = "postgres",
///     name = "PostgreSQL",
///     scheme = ConnectionUri,
///     protocol = PostgresProtocol,
///     icon = "postgres",
/// )]
/// pub struct PostgresCredential;
/// ```
#[proc_macro_derive(
    Credential,
    attributes(credential, oauth2, ldap, uses_resource, uses_credential)
)]
pub fn derive_credential(input: TokenStream) -> TokenStream {
    credential::derive(input)
}

/// Derive macro for the [`AuthScheme`] trait.
///
/// Generates an `impl AuthScheme` that returns the specified
/// [`AuthPattern`] variant. Types with custom `expires_at()` logic
/// (e.g., `OAuth2Token`, `Certificate`) should keep a manual impl.
///
/// # Errors
///
/// Emits a compile error when `#[auth_scheme(pattern = ...)]` is
/// missing or the pattern variant is not a valid `AuthPattern` identifier.
///
/// # Attributes
///
/// ## Container attributes (`#[auth_scheme(...)]` on the struct)
///
/// - `pattern = Variant` — the [`AuthPattern`] variant (required)
///
/// # Example
///
/// ```ignore
/// use nebula_credential::AuthScheme;
///
/// #[derive(Clone, Serialize, Deserialize, AuthScheme)]
/// #[auth_scheme(pattern = SecretToken)]
/// pub struct MyToken {
///     token: String,
/// }
/// ```
///
/// [`AuthScheme`]: https://docs.rs/nebula-credential/latest/nebula_credential/trait.AuthScheme.html
/// [`AuthPattern`]: https://docs.rs/nebula-credential/latest/nebula_credential/enum.AuthPattern.html
#[proc_macro_derive(AuthScheme, attributes(auth_scheme))]
pub fn derive_auth_scheme(input: TokenStream) -> TokenStream {
    auth_scheme::derive(input)
}
