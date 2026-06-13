//! Probe 2 (f): `SecretString` field on an external-tagged scheme.
//!
//! Macro audit must reject — `#[auth_scheme(external)]` schemes hold only a
//! handle to an out-of-process signer (HSM / KMS / FIDO), never secret bytes in
//! memory. A `SecretString` field means the secret IS in-process, so the author
//! must declare `sensitive` instead. This is the structural invariant that keeps
//! `external` from becoming a zeroize-bypass smuggling channel.

use nebula_credential::{AuthScheme, SecretString};

#[derive(AuthScheme)]
#[auth_scheme(pattern = Custom, family = SecretTokenFamily, external)]
struct BadExternalScheme {
    pub secret: SecretString, // SecretString on external-tagged scheme — REJECT
}

fn main() {}
