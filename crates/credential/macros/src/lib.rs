//! Proc-macro crate for the `Credential` derive macro and the
//! `#[capability]` attribute macro.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;

mod auth_scheme;
mod capability;
mod credential;
mod credential_attr;

/// Derive macro for the `Credential` trait — Phase 5 of the M6 dependency
/// redesign.
///
/// Mirrors `#[derive(Resource)]` (Phase 4) and `#[derive(Action)]` (Phase 3) on
/// the slot-binding family. Credentials are leaves of the dependency graph
/// (per spec 23 they cannot statically depend on other credentials), so the
/// macro's surface is intentionally smaller than the Resource / Action
/// variants — it emits the type-level `Credential` impl with `KEY`, the
/// associated types, the `metadata()` static, and the capability-report
/// const-bool impls (one per declared sub-trait flag).
///
/// # Modes
///
/// The macro supports two **mutually exclusive** modes for specifying
/// `Self::Properties`:
///
/// - **Properties mode** — `properties = TypePath` points at a `<Name>Properties` struct (typically
///   `#[derive(Schema, Deserialize)]`). The macro emits `type Properties = <TypePath>` and a
///   `todo!()` resolver; for non-trivial credentials, prefer writing the entire `impl Credential
///   for X` by hand rather than mixing the derive with manual overrides (Rust coherence forbids
///   splitting a single trait impl).
/// - **Protocol mode** — `protocol = TypePath` points at a `StaticProtocol` impl. The macro emits
///   `type Properties = <TypePath as StaticProtocol>::Properties` and a resolver body that
///   delegates to `<TypePath as StaticProtocol>::build`. The canonical ergonomic path for static
///   (non-interactive) credentials.
///
/// # Container attributes (`#[credential(...)]`)
///
/// - `key = "..."` — Unique credential type key (required).
/// - `name = "..."` — Human-readable name (required).
/// - `scheme = TypePath` — Auth scheme produced by `Credential::project`; doubles as `State`
///   (identity-state pattern). Required.
/// - `properties = TypePath` — Direct path to the `<Name>Properties` companion struct (required if
///   `protocol` is absent).
/// - `protocol = TypePath` — `StaticProtocol` impl (required if `properties` is absent).
/// - `icon = "..."` — Catalog icon identifier (optional).
/// - `doc_url = "..."` — Documentation URL (optional).
/// - `capabilities(...)` — Sub-traits the credential implements: any subset of `interactive`,
///   `refreshable`, `revocable`, `testable`, `dynamic`. Per Tech Spec §15.8 (closes security-lead
///   N6) the macro emits one `plugin_capability_report::IsX` const-bool impl per capability — and a
///   parity assertion that consumes the actual sub-trait bound, so a missing `impl Refreshable for
///   X` fails to compile.
///
/// # Outer struct attributes
///
/// - `#[uses_resource(TypeName, purpose = "...")]` — Declare a resource dependency (repeatable).
/// - `#[uses_credential(...)]` — Forbidden: credential-to-credential static dependencies are not
///   allowed (spec 23). Emits a compile error.
///
/// # Examples
///
/// Protocol mode (the canonical static-credential path):
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
///
/// Properties mode (schema-only bridging):
/// ```ignore
/// use nebula_credential::Credential;
/// use nebula_schema::Schema;
/// use serde::Deserialize;
///
/// #[derive(Schema, Deserialize)]
/// pub struct GithubOAuthProperties {
///     #[field(label = "Client ID")]
///     #[validate(required)]
///     pub client_id: String,
/// }
///
/// #[derive(Credential)]
/// #[credential(
///     key = "github_oauth",
///     name = "GitHub OAuth",
///     scheme = OAuth2Token,
///     properties = GithubOAuthProperties,
/// )]
/// pub struct GithubOAuthCredential;
/// ```
#[proc_macro_derive(
    Credential,
    attributes(credential, oauth2, ldap, uses_resource, uses_credential)
)]
pub fn derive_credential(input: TokenStream) -> TokenStream {
    credential::derive(input)
}

/// Attribute macro for declaring a credential as a single `impl` block
/// (ADR-0088 D1 — the canonical authoring path, superseding
/// `#[derive(Credential)]`).
///
/// Applied to an inherent `impl Type { … }`, it reads the associated types
/// and the methods present and emits the base `Credential` impl, one
/// capability sub-trait impl per capability **method** supplied (`refresh` ⇒
/// `Refreshable`, `revoke` ⇒ `Revocable`, `test` ⇒ `Testable`,
/// `continue_resolve` ⇒ `Interactive`, `release` ⇒ `Dynamic`), the five
/// `plugin_capability_report::IsX` consts, and a
/// `CredentialLifecycle::policy()` derived from those same methods.
///
/// Capability is **inferred from method presence**, never declared — so the
/// capability-report consts and the lifecycle policy can never disagree with
/// the implemented capabilities (the `E0046` compile-gate is preserved while
/// the four old declaration sites collapse into one).
///
/// # Arguments
///
/// - `key = "…"` — stable credential type key (required).
/// - `name = "…"` — required only when no `fn metadata` is supplied.
/// - `description = "…"`, `icon = "…"`, `doc_url = "…"` — optional metadata.
///
/// # Example
///
/// ```ignore
/// use nebula_credential::credential;
///
/// #[credential(key = "api_key", name = "API Key", icon = "key")]
/// impl ApiKeyCredential {
///     type Properties = ApiKeyProperties;
///     type Scheme = SecretToken;
///     type State = SecretToken;
///
///     fn project(state: &SecretToken) -> SecretToken { state.clone() }
///
///     async fn resolve(values: &FieldValues, _ctx: &CredentialContext)
///         -> Result<ResolveResult<SecretToken, ()>, CredentialError> { /* … */ }
/// }
/// ```
///
/// # Errors
///
/// Emits a compile error when applied to a trait impl, when a required item
/// (`type Properties`/`Scheme`/`State`, `fn project`/`resolve`) is missing,
/// when `continue_resolve` and `type Pending` are not supplied as a pair, or
/// when an unrecognized item appears (a typo cannot silently drop a
/// capability — move inherent helpers to a separate `impl` block).
#[proc_macro_attribute]
pub fn credential(args: TokenStream, input: TokenStream) -> TokenStream {
    credential_attr::expand(args, input)
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
/// capability macro canonical form: real trait, service/scheme blanket impl,
/// sealed-blanket, phantom trait, and phantom blanket. Hides the
/// two-trait verbosity from everyday plugin and built-in code.
///
/// # Arguments
///
/// - `scheme_bound = <Path>` - the marker trait the credential's `Scheme` associated type must
///   satisfy (e.g. `AcceptsBearer`).
/// - `sealed = <Ident>` - the per-capability inner sealed trait inside the crate-root `mod
///   sealed_caps` (e.g. `BearerSealed`). The crate author must declare this module manually; see
///   capability macro 4.1 / 4.2.
///
/// # Visibility
///
/// The emitted phantom trait inherits the visibility of the capability
/// trait (visibility-symmetry, per capability macro §1 amendment 2026-04-26).
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
/// as `E0433` at the emitted blanket impl line per capability macro 4.1.
#[proc_macro_attribute]
pub fn capability(args: TokenStream, input: TokenStream) -> TokenStream {
    capability::expand(args, input)
}
