//! Proc-macro crate for the `Credential` derive macro and the
//! `#[capability]` attribute macro.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod auth_scheme;
mod capability;
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
/// - `capabilities(...)` - List of sub-traits the credential implements (optional, default empty);
///   accepts `interactive`, `refreshable`, `revocable`, `testable`, `dynamic`. Per Tech Spec §15.8
///   (closes security-lead N6) the macro emits one `plugin_capability_report::IsX` const-bool impl
///   per capability — `true` for listed flags, `false` for the rest — so
///   `CredentialRegistry::capabilities_of` matches the actual sub-trait surface rather than a
///   self-attested metadata field.
///
/// ## Dependency attributes (outer attributes on the struct)
///
/// - `#[uses_resource(TypeName, purpose = "...")]` - Declare a resource dependency (repeatable)
/// - `#[uses_credential(...)]` - **Forbidden** - emits a compile error (spec 23)
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

/// Derive macro for the `AuthScheme` trait.
///
/// Generates an `impl AuthScheme` that returns the specified
/// `AuthPattern` variant. Types with custom `expires_at()` logic
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
/// - `pattern = Variant` - the `AuthPattern` variant (required)
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
#[proc_macro_derive(AuthScheme, attributes(auth_scheme))]
pub fn derive_auth_scheme(input: TokenStream) -> TokenStream {
    auth_scheme::derive(input)
}

/// Attribute macro for declaring a capability sub-trait.
///
/// Expands a single capability trait declaration into the full
/// ADR-0035 canonical form: real trait, service/scheme blanket impl,
/// sealed-blanket, phantom trait, and phantom blanket. Hides the
/// two-trait verbosity from everyday plugin and built-in code.
///
/// # Arguments
///
/// - `scheme_bound = <Path>` - the marker trait the credential's `Scheme` associated type must
///   satisfy (e.g. `AcceptsBearer`).
/// - `sealed = <Ident>` - the per-capability inner sealed trait inside the crate-root `mod
///   sealed_caps` (e.g. `BearerSealed`). The crate author must declare this module manually; see
///   ADR-0035 4.1 / 4.2.
///
/// # Visibility
///
/// The emitted phantom trait inherits the visibility of the capability
/// trait (visibility-symmetry, per ADR-0035 §1 amendment 2026-04-26).
/// `pub trait Cap` produces `pub trait CapPhantom`; `pub(crate) trait
/// Cap` produces `pub(crate) trait CapPhantom`. This composes correctly
/// with crate-internal capabilities — forcing the phantom to a fixed
/// visibility would leak crate-private capabilities through their
/// phantoms onto the public surface.
///
/// ```ignore
/// // Crate-internal capability — phantom is also pub(crate).
/// #[nebula_credential_macros::capability(scheme_bound = AcceptsBearer, sealed = LocalSealed)]
/// pub(crate) trait LocalCapability: LocalService {}
/// // Emits: pub(crate) trait LocalCapabilityPhantom: …
///
/// // Public capability — phantom is also pub.
/// #[nebula_credential_macros::capability(scheme_bound = AcceptsBearer, sealed = BearerSealed)]
/// pub trait BitbucketBearer: BitbucketCredential {}
/// // Emits: pub trait BitbucketBearerPhantom: …
/// ```
///
/// # Example
///
/// ```ignore
/// // Crate-root module - declared once by the crate author.
/// mod sealed_caps {
///     pub trait BearerSealed {}
///     pub trait BasicSealed {}
/// }
///
/// // Service supertrait declared elsewhere in the same crate.
/// pub trait BitbucketCredential: nebula_credential::Credential {}
///
/// // Capability declaration.
/// #[nebula_credential_macros::capability(scheme_bound = AcceptsBearer, sealed = BearerSealed)]
/// pub trait BitbucketBearer: BitbucketCredential {}
/// ```
///
/// # Errors
///
/// Emits a compile error when:
///
/// - Either argument is missing or repeated.
/// - The trait body is non-empty (capability traits are markers).
/// - The trait has zero or multiple non-marker supertraits.
///
/// A missing `mod sealed_caps` or missing inner sealed trait surfaces
/// as `E0433` at the emitted blanket impl line per ADR-0035 4.1.
#[proc_macro_attribute]
pub fn capability(args: TokenStream, input: TokenStream) -> TokenStream {
    capability::expand(args, input)
}
