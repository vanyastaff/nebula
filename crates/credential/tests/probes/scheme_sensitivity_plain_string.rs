//! Probe 2 (a): plain `String` for token-named field on a sensitive-tagged scheme.
//!
//! Macro audit must reject this — secret-named fields require
//! `SecretString` / `SecretBytes` / nested `SensitiveScheme`.

use nebula_credential::AuthScheme;

#[derive(AuthScheme)]
#[auth_scheme(pattern = SecretToken, sensitive)]
struct BadScheme {
    pub token: String, // plain String for sensitive-tagged field — REJECT
}

fn main() {}
