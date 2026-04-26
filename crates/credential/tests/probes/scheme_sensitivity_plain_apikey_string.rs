//! Probe 2 (e): snake_case secret-named plain `String` on a sensitive scheme.
//!
//! `api_key`, `client_secret`, `access_token`, `bearer_token`, etc. all
//! contain a secret-marker segment. The word-segment match in
//! `is_secret_named` must reject plain `String` for any such field.

use nebula_credential::AuthScheme;

#[derive(AuthScheme)]
#[auth_scheme(pattern = ApiKey, sensitive)]
struct BadSnakeCase {
    pub api_key: String, // snake_case secret-named plain String — REJECT
}

fn main() {}
