//! First-party credential registration entry point.
//!
//! `nebula-credential-runtime` (and any composition root) calls
//! [`register_builtins`] to add the first-party reference credential
//! types to a [`CredentialRegistry`] alongside the contract crate's own
//! types and plugin-discovered types.

use nebula_credential::CredentialRegistry;
use nebula_credential::contract::registry::RegisterError;

use crate::{BearerTokenCredential, SharedKeyCredential, SigningKeyCredential};

/// Register every first-party reference credential into `registry`.
///
/// Fail-closed on duplicate KEY (Tech Spec §15.6): if a KEY is already
/// present the second registration is **rejected** with
/// [`RegisterError::DuplicateKey`], the first registration remains
/// authoritative, and `registry` is left unchanged for the rejected
/// entry. This is not silent "first-wins" — the collision surfaces as an
/// error the caller must handle.
///
/// # Errors
///
/// Returns [`RegisterError::DuplicateKey`] if any reference KEY is
/// already present in `registry` (e.g. a plugin shipped a colliding KEY).
pub fn register_builtins(registry: &mut CredentialRegistry) -> Result<(), RegisterError> {
    let crate_name = env!("CARGO_CRATE_NAME");
    registry.register(BearerTokenCredential, crate_name)?;
    registry.register(SharedKeyCredential, crate_name)?;
    registry.register(SigningKeyCredential, crate_name)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::register_builtins;
    use nebula_credential::CredentialRegistry;

    #[test]
    fn registers_all_three_reference_credentials() {
        let mut reg = CredentialRegistry::new();
        register_builtins(&mut reg).expect("register_builtins ok");
        assert_eq!(reg.len(), 3);
        assert!(reg.contains("bearer_token"));
        assert!(reg.contains("shared_key"));
        assert!(reg.contains("signing_key"));
    }

    #[test]
    fn register_builtins_is_idempotent_safe_on_fresh_registry() {
        let mut a = CredentialRegistry::new();
        let mut b = CredentialRegistry::new();
        register_builtins(&mut a).expect("a ok");
        register_builtins(&mut b).expect("b ok");
        assert_eq!(a.len(), 3);
        assert_eq!(b.len(), 3);
    }
}
