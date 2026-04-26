//! Probe 2 (b): `SecretString` field on a public-tagged scheme.
//!
//! Macro audit must reject — `#[auth_scheme(public)]` schemes hold no
//! secret material; the author must declare `sensitive` instead.

use nebula_credential::{AuthScheme, SecretString};

#[derive(AuthScheme)]
#[auth_scheme(pattern = Custom, public)]
struct BadPublicScheme {
    pub secret: SecretString, // SecretString on public-tagged scheme — REJECT
}

fn main() {}
