//! Probe — SEC-05 hardening: `CredentialGuard<S>: !Clone`.
//!
//! FQS prevents resolution through `Deref<Target = S>` to the inner
//! `S::clone()`. `CredentialGuard` itself does not implement `Clone`, so
//! the trait-level call fails with `E0277`.

use nebula_credential::CredentialGuard;
use zeroize::Zeroize;

#[derive(Clone)]
struct DummySecret(String);

impl Zeroize for DummySecret {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

fn try_clone_via_fqs(g: &CredentialGuard<DummySecret>) -> CredentialGuard<DummySecret> {
    // FQS forces trait resolution on the guard, not on `&S` via auto-deref.
    // `CredentialGuard` no longer implements `Clone` (SEC-05), so this is
    // rejected with `E0277` at compile time.
    <CredentialGuard<DummySecret> as Clone>::clone(g)
}

fn main() {
    let _ = try_clone_via_fqs;
}
