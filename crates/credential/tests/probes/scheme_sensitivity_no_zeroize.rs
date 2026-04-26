//! Probe 2 (c): manual `impl SensitiveScheme` without `ZeroizeOnDrop`.
//!
//! Trait bound `SensitiveScheme: AuthScheme + ZeroizeOnDrop` rejects
//! at the impl site with E0277 — no macro audit needed.

use nebula_credential::{AuthScheme, SecretString, SensitiveScheme};

// Manual impl that skips ZeroizeOnDrop
struct ManualScheme {
    pub token: SecretString,
}

impl AuthScheme for ManualScheme {
    fn pattern() -> nebula_credential::AuthPattern {
        nebula_credential::AuthPattern::SecretToken
    }
}

impl SensitiveScheme for ManualScheme {} // E0277 — ZeroizeOnDrop not satisfied

fn main() {}
