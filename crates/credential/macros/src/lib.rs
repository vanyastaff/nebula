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

/// Derive macro for the [`AuthScheme`] trait + sensitivity sub-trait.
///
/// Generates `impl AuthScheme` returning the specified [`AuthPattern`]
/// variant, and one of `impl SensitiveScheme` or `impl PublicScheme`
/// per Tech Spec §15.5 dichotomy. The macro also audits scheme fields
/// against the declared sensitivity at expansion time.
///
/// # Errors
///
/// Emits a compile error when:
/// - `#[auth_scheme(pattern = ...)]` is missing or the pattern variant is not a valid `AuthPattern`
///   identifier.
/// - Neither `sensitive` nor `public` is declared, or both are declared.
/// - `sensitive` is declared but a secret-named field (token / secret / key / password / bearer) is
///   typed as plain `String` / `Vec<u8>`.
/// - `public` is declared but any field is typed as `SecretString` / `SecretBytes`.
///
/// # Attributes
///
/// ## Container attributes (`#[auth_scheme(...)]` on the struct)
///
/// - `pattern = Variant` — the [`AuthPattern`] variant (required)
/// - `sensitive` — declares scheme holds secret material; mandates `ZeroizeOnDrop` (enforced via
///   `SensitiveScheme: AuthScheme + ZeroizeOnDrop` trait bound — derive `Zeroize`+`ZeroizeOnDrop`).
/// - `public` — declares scheme holds no secret material; field audit forbids
///   `SecretString`/`SecretBytes`.
///
/// `sensitive` and `public` are mutually exclusive; exactly one must be
/// declared.
///
/// # Example — sensitive scheme
///
/// ```ignore
/// use nebula_credential::{AuthScheme, SecretString};
///
/// #[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop, AuthScheme)]
/// #[auth_scheme(pattern = SecretToken, sensitive)]
/// pub struct MyToken {
///     token: SecretString,
/// }
/// ```
///
/// # Example — public scheme
///
/// ```ignore
/// use nebula_credential::AuthScheme;
///
/// #[derive(Clone, Serialize, Deserialize, AuthScheme)]
/// #[auth_scheme(pattern = InstanceIdentity, public)]
/// pub struct MyBinding {
///     provider: String,
///     role: String,
/// }
/// ```
///
/// [`AuthScheme`]: https://docs.rs/nebula-credential/latest/nebula_credential/trait.AuthScheme.html
/// [`AuthPattern`]: https://docs.rs/nebula-credential/latest/nebula_credential/enum.AuthPattern.html
#[proc_macro_derive(AuthScheme, attributes(auth_scheme))]
pub fn derive_auth_scheme(input: TokenStream) -> TokenStream {
    auth_scheme::derive(input)
}
