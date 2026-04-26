//! Probe 7 — `SchemeGuard<'a, C>: !Clone`.
//!
//! Plaintext scheme material must not be silently duplicated. The fixture
//! invokes `Clone::clone` via fully-qualified syntax so the compiler
//! cannot autoderef to the wrapped `Scheme` (which may itself be `Clone`
//! for unrelated reasons). The missing `Clone` impl on `SchemeGuard` is
//! rejected with `E0277` (the trait `Clone` is not implemented).

use nebula_credential::credentials::ApiKeyCredential;
use nebula_credential::SchemeGuard;

fn try_clone_via_fqs<'a>(
    g: &SchemeGuard<'a, ApiKeyCredential>,
) -> SchemeGuard<'a, ApiKeyCredential> {
    // Fully-qualified syntax forces trait resolution on the guard
    // itself, not on the deref'd inner Scheme. `SchemeGuard` does not
    // implement `Clone`, so this is rejected at compile time.
    <SchemeGuard<'a, ApiKeyCredential> as Clone>::clone(g)
}

fn main() {
    // Reference the function so the failure is attached to it rather
    // than dead-coded away by codegen.
    let _ = try_clone_via_fqs;
}
