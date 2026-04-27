//! Probe — SEC-06 hardening: `SchemeGuard<'a, C>: !Send`.
//!
//! Asserting `Send` on a function bound that captures a `SchemeGuard`
//! fails because the guard contains `PhantomData<*const ()>` which
//! propagates `!Send`. The bound is asserted statically via a generic
//! function that requires `Send`.

use nebula_credential::SchemeGuard;
use nebula_credential::credentials::ApiKeyCredential;

fn require_send<T: Send>(_: T) {}

fn try_send_guard<'a>(g: SchemeGuard<'a, ApiKeyCredential>) {
    // E0277: `*const ()` cannot be sent between threads safely.
    // (The `*const ()` lives inside `_thread_marker: PhantomData<*const ()>`
    // on `SchemeGuard`, propagated through the type's auto-trait derivation.)
    require_send(g);
}

fn main() {
    let _ = try_send_guard;
}
