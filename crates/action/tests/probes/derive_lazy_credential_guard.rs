//! Compile-fail probe: lazy credential slots never deferred acquisition.
use nebula_action::Action;
use nebula_core::sync::Lazy;
use nebula_credential::{AuthScheme, CredentialGuard};

#[derive(Action)]
#[action(key = "bad.lazy_credential", input = serde_json::Value, output = serde_json::Value)]
struct LazyCredential<S: AuthScheme + zeroize::Zeroize> {
    #[credential]
    auth: Lazy<CredentialGuard<S>>,
}

fn main() {}
