//! Probe — the F3 moat: a credential of one protocol cannot be bound where
//! another protocol's scheme is expected.
//!
//! A resource slot is typed `SlotCell<CredentialGuard<Scheme>>`; the safety is
//! that `CredentialGuard<S>` is **nominal**, so a `CredentialGuard<TwilioScheme>`
//! cannot stand in for a `CredentialGuard<StripeScheme>`. This fixture models a
//! Stripe-typed slot sink and feeds it a Twilio guard — expects `E0308`.

use nebula_credential::{AuthPattern, AuthScheme, CredentialGuard, SecretTokenFamily};
use zeroize::Zeroize;

#[derive(Zeroize)]
struct StripeScheme {
    secret: String,
}
impl AuthScheme for StripeScheme {
    type Family = SecretTokenFamily;
    fn pattern() -> AuthPattern {
        AuthPattern::SecretToken
    }
}

#[derive(Zeroize)]
struct TwilioScheme {
    secret: String,
}
impl AuthScheme for TwilioScheme {
    type Family = SecretTokenFamily;
    fn pattern() -> AuthPattern {
        AuthPattern::SecretToken
    }
}

/// Stands in for a Stripe-typed slot sink (`SlotCell<CredentialGuard<StripeScheme>>::store`).
fn bind_stripe_slot(_guard: CredentialGuard<StripeScheme>) {}

fn main() {
    let twilio = CredentialGuard::new(TwilioScheme {
        secret: String::new(),
    });
    // E0308 — expected `CredentialGuard<StripeScheme>`, found `CredentialGuard<TwilioScheme>`.
    bind_stripe_slot(twilio);
}
