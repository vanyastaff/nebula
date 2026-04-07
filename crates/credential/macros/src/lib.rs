//! Proc-macro crate for the `Credential` derive macro.
//!
//! Generates a Credential impl for static (non-interactive) credentials
//! backed by a StaticProtocol.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

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
///     scheme = DatabaseAuth,
///     protocol = DatabaseProtocol,
///     icon = "postgres",
/// )]
/// pub struct PostgresCredential;
/// ```
#[proc_macro_derive(Credential, attributes(credential, oauth2, ldap))]
pub fn derive_credential(input: TokenStream) -> TokenStream {
    credential::derive(input)
}
