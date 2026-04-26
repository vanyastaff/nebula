//! Probe (Stage-8 PR #582 review followup) — capability self-attestation.
//!
//! `#[derive(Credential)]` emits `IsRefreshable::VALUE = true` purely
//! from the `#[credential(capabilities(refreshable))]` attribute, which
//! silently re-introduced the §15.8 self-attestation anti-pattern: a
//! credential could advertise refresh capability without an `impl
//! Refreshable for X`. The macro now emits a parity assertion alongside
//! the IsX impl that consumes the actual sub-trait bound, so a missing
//! `impl Refreshable for ApiKeyCredential` fails at expansion with
//! `the trait bound \`ApiKeyCredential: Refreshable\` is not satisfied`.

use nebula_credential::{Credential, StaticProtocol, error::CredentialError, scheme::SecretToken};
use nebula_schema::FieldValues;

struct DummyProtocol;

impl StaticProtocol for DummyProtocol {
    type Input = FieldValues;
    type Scheme = SecretToken;

    fn build(_values: &FieldValues) -> Result<SecretToken, CredentialError> {
        unimplemented!("fixture")
    }
}

#[derive(nebula_credential_macros::Credential)]
#[credential(
    key = "dummy",
    name = "dummy",
    scheme = SecretToken,
    protocol = DummyProtocol,
    capabilities(refreshable)
)]
struct DummyCredential;

// Note: deliberately no `impl Refreshable for DummyCredential` — the
// parity assertion in the derive must reject this at compile time.

fn main() {
    fn assert_credential<C: Credential>() {}
    assert_credential::<DummyCredential>();
}
