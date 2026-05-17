//! Spec §6 #3 (structural half): no secret echo in responses.
//!
//! `CredentialSnapshot` is the facade's response/inspection type. It
//! deliberately does **not** implement `serde::Serialize` (it holds a
//! type-erased `Box<dyn Any>` projected scheme), so a secret-bearing
//! projection can never be serialized onto the wire by serde — not even
//! by an accidental `#[derive(Serialize)]` on a wrapping DTO.
//!
//! This probe forces a `T: Serialize` bound on `CredentialSnapshot`.
//! Expected: a trait-bound error (E0277 — `Serialize` is not implemented
//! for `CredentialSnapshot`), proving the property structurally.

use nebula_credential::{CredentialRecord, CredentialSnapshot};

fn requires_serialize<T: serde::Serialize>(_: &T) {}

fn main() {
    let snapshot = CredentialSnapshot::new(
        "bearer_token",
        CredentialRecord::new(),
        nebula_credential::scheme::SecretToken::new(nebula_credential::SecretString::new(
            "sk-secret",
        )),
    );
    // Must NOT compile: CredentialSnapshot: !Serialize by design.
    requires_serialize(&snapshot);
}
