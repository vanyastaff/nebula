//! Probe 6 — `SchemeGuard<'a, C>` cannot be retained past the call.
//!
//! Per Tech Spec §15.7 spike iter-3 secondary finding: when the engine
//! invokes a refresh hook, it passes `SchemeGuard<'a, C>` alongside a
//! borrow that shares `'a`. A retention attempt forces the shared borrow
//! to outlive the call, which the borrow checker rejects.
//!
//! This fixture stores a `SchemeGuard<'a, C>` into a struct field whose
//! storage lifetime is `'static` (via the `Option<SchemeGuard<'static, _>>`
//! field type). The borrow checker rejects the assignment because the
//! guard's call-site lifetime `'a` cannot be widened to `'static`.

use nebula_credential::credentials::ApiKeyCredential;
use nebula_credential::{CredentialContext, SchemeGuard};

/// A "leaky" resource that wants to stash a fresh scheme into a long-lived
/// slot. The field type is `SchemeGuard<'static, _>` because the resource
/// itself is long-lived, but the engine only ever hands out call-bound
/// `'a` guards — the assignment must fail.
struct LeakyResource {
    stored: Option<SchemeGuard<'static, ApiKeyCredential>>,
}

impl LeakyResource {
    fn try_retain<'a>(
        &mut self,
        new_scheme: SchemeGuard<'a, ApiKeyCredential>,
        // `ctx` is decorative for this probe — it mirrors the canonical
        // refresh-hook signature (`Resource::on_credential_refresh` takes a
        // `&'a CredentialContext` alongside the guard per ADR-0036 + Tech
        // Spec §15.4) so the probe matches the production call shape
        // verbatim. The retention error would fire on `new_scheme` alone;
        // the parameter is kept to make the fixture grep-equivalent to
        // real call sites and to ensure any future refactor of the hook
        // signature surfaces here.
        ctx: &'a CredentialContext,
    ) {
        let _ = ctx;
        // E0521 / E0597 — `new_scheme` carries lifetime `'a` from the
        // call site; storing it in `self.stored: Option<SchemeGuard<'static, _>>`
        // would require widening `'a` to `'static`, which is forbidden
        // because the engine borrow shares `'a`.
        self.stored = Some(new_scheme);
    }
}

fn main() {
    // Reference the function so the failure attaches to it.
    let _ = LeakyResource::try_retain;
}
