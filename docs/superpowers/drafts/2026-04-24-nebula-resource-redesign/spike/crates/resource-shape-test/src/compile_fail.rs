//! Compile-fail probes — encoded as `compile_fail` doctests.
//!
//! `cargo test --doc` runs these. The compiler MUST reject each block;
//! passing means the spike trait shape lets through something it
//! shouldn't.
//!
//! Why doctests instead of `trybuild`: keeps the spike workspace dep
//! tree minimal and avoids a separate test harness. The trade-off is
//! that diagnostics are less precise — we only get the binary
//! "did it compile?" answer. That's enough for spike sanity.

/// **MUST FAIL** — overriding `on_credential_refresh` with the wrong
/// signature (different argument type) is a compile error. This proves
/// the trait actually constrains overrides to the projected scheme type;
/// a permissive trait that accepted any signature would let production
/// resources silently mis-handle rotation.
///
/// ```compile_fail
/// use std::future::Future;
///
/// use nebula_credential::{
///     AuthPattern, AuthScheme, Credential, CredentialContext, CredentialError,
///     CredentialMetadata, CredentialState, NoPendingState, ResolveResult,
///     credential_key,
/// };
/// use nebula_schema::HasSchema;
/// use resource_shape::{NoCredential, Resource, ResourceContext, ResourceKey};
///
/// struct WrongSig;
/// #[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
/// struct WrongScheme;
/// impl AuthScheme for WrongScheme {
///     fn pattern() -> AuthPattern {
///         AuthPattern::NoAuth
///     }
/// }
/// impl CredentialState for WrongScheme {
///     const KIND: &'static str = "wrong";
///     const VERSION: u32 = 1;
/// }
/// struct WrongCred;
/// impl Credential for WrongCred {
///     type Input = ();
///     type Scheme = WrongScheme;
///     type State = WrongScheme;
///     type Pending = NoPendingState;
///     const KEY: &'static str = "wrong";
///     fn metadata() -> CredentialMetadata
///     where Self: Sized {
///         CredentialMetadata::builder()
///             .key(credential_key!("wrong"))
///             .name("w").description("w")
///             .schema(<() as HasSchema>::schema())
///             .pattern(AuthPattern::NoAuth)
///             .build().unwrap()
///     }
///     fn project(_state: &WrongScheme) -> WrongScheme { WrongScheme }
///     fn resolve(
///         _values: &nebula_schema::FieldValues,
///         _ctx: &CredentialContext,
///     ) -> impl Future<Output = Result<ResolveResult<WrongScheme, NoPendingState>, CredentialError>> + Send {
///         async { Ok(ResolveResult::Complete(WrongScheme)) }
///     }
/// }
///
/// impl Resource for WrongSig {
///     type Config = ();
///     type Runtime = ();
///     type Lease = ();
///     type Error = std::io::Error;
///     type Credential = WrongCred;
///
///     fn key() -> ResourceKey { ResourceKey("wrong") }
///
///     fn create(
///         &self,
///         _config: &Self::Config,
///         _scheme: &<WrongCred as Credential>::Scheme,
///         _ctx: &ResourceContext,
///     ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send {
///         async { Ok(()) }
///     }
///
///     // Wrong: takes &str instead of &Scheme. Must NOT compile —
///     // this method signature does not match the trait's.
///     fn on_credential_refresh(
///         &self,
///         _new_scheme: &str,
///     ) -> impl Future<Output = Result<(), Self::Error>> + Send {
///         async { Ok(()) }
///     }
/// }
/// ```
#[allow(dead_code)]
pub fn _wrong_refresh_signature_must_fail() {}

/// **MUST FAIL** — using `type Credential = NoCredential;` and then
/// trying to *reach into* the scheme value with anything that isn't
/// `NoScheme` should not type-check. (NoScheme is the projected scheme
/// of NoCredential.)
///
/// ```compile_fail
/// use resource_shape::{NoCredential, NoScheme};
/// use nebula_credential::Credential;
///
/// fn _check() {
///     // Using NoScheme as if it were a SecretToken — must fail.
///     let _scheme: <NoCredential as Credential>::Scheme = NoScheme;
///     // This line attempts to call a SecretToken method on NoScheme.
///     // The compiler should reject — NoScheme has no `.token()`.
///     let _bad = _scheme.token();
/// }
/// ```
#[allow(dead_code)]
pub fn _no_credential_scheme_is_inert_must_fail() {}

/// **MUST FAIL** — registering a Resource whose `Self::Credential` does
/// not satisfy `Credential` is a compile error. Demonstrates that the
/// `type Credential: Credential` bound bites at impl time, not just at
/// dispatch.
///
/// ```compile_fail
/// use std::future::Future;
/// use resource_shape::{Resource, ResourceContext, ResourceKey};
///
/// struct NotACredential;
///
/// struct BadResource;
/// impl Resource for BadResource {
///     type Config = ();
///     type Runtime = ();
///     type Lease = ();
///     type Error = std::io::Error;
///     // Wrong: NotACredential does not impl Credential. Must NOT compile.
///     type Credential = NotACredential;
///
///     fn key() -> ResourceKey { ResourceKey("bad") }
///
///     fn create(
///         &self,
///         _config: &Self::Config,
///         _scheme: &(),
///         _ctx: &ResourceContext,
///     ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send {
///         async { Ok(()) }
///     }
/// }
/// ```
#[allow(dead_code)]
pub fn _credential_bound_enforced_must_fail() {}
