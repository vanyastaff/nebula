//! Probe 6 analogue — `Resource::on_credential_refresh` cannot retain
//! `SchemeGuard<'a, _>` past the engine call.
//!
//! Mirrors the credential-side Probe 6 (`scheme_guard_retention.rs`) at
//! the `Resource` trait layer per `nebula-resource` П2 plan Task 13. If a
//! future trait reshape weakens the `'a` lifetime bound on
//! `Resource::on_credential_refresh<'a>(&self, SchemeGuard<'a, _>,
//! &'a CredentialContext) -> impl Future + 'a`, this probe will start
//! compiling and the trybuild driver will fail loudly.
//!
//! The fixture is a `LeakyResource` whose body tries to stash the
//! call-bound `SchemeGuard<'a, _>` into an `Option<SchemeGuard<'static, _>>`
//! field. The borrow checker rejects the assignment because the guard's
//! call-site lifetime `'a` cannot be widened to `'static` — the engine
//! borrow shares `'a` and would otherwise have to outlive the resource.

use std::future::Future;

use nebula_credential::credentials::ApiKeyCredential;
use nebula_credential::{Credential, CredentialContext, SchemeGuard};
use nebula_resource::{Resource, ResourceConfig, ResourceContext};
use nebula_resource::{ResourceKey, resource_key};
use nebula_schema::HasSchema;

/// Local config newtype — `()` has a baseline `HasSchema` impl but no blanket
/// `ResourceConfig` impl, and orphan rules prevent us from wiring one up
/// from this fixture. Defining a local type makes `Config = LeakyConfig`
/// well-formed so the only compile error surfaced is the lifetime
/// retention failure we are asserting against.
#[derive(Clone)]
struct LeakyConfig;

impl HasSchema for LeakyConfig {
    fn schema() -> nebula_schema::ValidSchema {
        <() as HasSchema>::schema()
    }
}

impl ResourceConfig for LeakyConfig {}

#[derive(Debug)]
struct LeakyError;

impl std::fmt::Display for LeakyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "leaky")
    }
}

impl std::error::Error for LeakyError {}

impl From<LeakyError> for nebula_resource::Error {
    fn from(_: LeakyError) -> Self {
        nebula_resource::Error::permanent("leaky")
    }
}

/// A "leaky" resource whose `on_credential_refresh` body tries to stash
/// the fresh `SchemeGuard` into a `'static`-lifetimed slot. The field
/// type is `Option<SchemeGuard<'static, _>>` because the resource itself
/// is long-lived (`'static`), but the engine only ever hands out
/// call-bound `'a` guards — the assignment must fail.
struct LeakyResource {
    stash: std::sync::Mutex<Option<SchemeGuard<'static, ApiKeyCredential>>>,
}

impl Resource for LeakyResource {
    type Config = LeakyConfig;
    type Runtime = ();
    type Lease = ();
    type Error = LeakyError;
    type Credential = ApiKeyCredential;

    fn key() -> ResourceKey {
        resource_key!("leaky")
    }

    fn create(
        &self,
        _config: &Self::Config,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send {
        async { Ok(()) }
    }

    fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        _ctx: &'a CredentialContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
        async move {
            // E0521 / E0597 — `new_scheme` carries lifetime `'a` from the
            // engine call site; storing it in
            // `self.stash: Option<SchemeGuard<'static, _>>` would require
            // widening `'a` to `'static`, which is forbidden because the
            // engine borrow shares `'a`.
            let mut stash = self.stash.lock().unwrap();
            *stash = Some(new_scheme);
            Ok(())
        }
    }
}

fn main() {
    // Reference the type so the failure attaches to the impl.
    let _ = std::mem::size_of::<LeakyResource>();
}
