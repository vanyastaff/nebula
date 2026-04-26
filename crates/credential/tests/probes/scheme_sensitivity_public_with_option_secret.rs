//! Probe 2 (d): `Option<SecretString>` field on a public-tagged scheme.
//!
//! Macro audit must look through `Option<T>` / `Box<T>` / `Arc<T>` / `Rc<T>`
//! wrappers — otherwise `pub maybe: Option<SecretString>` slips past the
//! `public` audit silently. The trait bound `PublicScheme: AuthScheme`
//! gives no friendly diagnostic for this case.

use nebula_credential::{AuthScheme, SecretString};

#[derive(AuthScheme)]
#[auth_scheme(pattern = Custom, public)]
struct BadOptionalSecret {
    pub maybe: Option<SecretString>, // Option<SecretString> on public — REJECT
}

fn main() {}
