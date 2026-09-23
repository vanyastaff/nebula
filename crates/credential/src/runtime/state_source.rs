//! Credential material source selected by application composition.
//!
//! Local encrypted persistence is the implemented service/projection path.
//! External providers have a contract but are not wired into that path; selecting
//! one fails closed rather than falling back to local material.

use std::sync::Arc;

use nebula_credential::provider::ExternalProvider;

/// Where a credential's resolved material comes from.
#[derive(Default)]
#[non_exhaustive]
pub enum StateSource {
    /// The crate-private layered encrypted store (default).
    #[default]
    LocalEncrypted,
    /// An external secret provider chain (Vault, etc.). Service and projection
    /// resolution reject this source until its provider/lease bridge is wired.
    External(Arc<dyn ExternalProvider>),
}

impl std::fmt::Debug for StateSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalEncrypted => f.write_str("StateSource::LocalEncrypted"),
            Self::External(p) => f
                .debug_tuple("StateSource::External")
                .field(&p.provider_name())
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StateSource;

    #[test]
    fn default_is_local_encrypted() {
        assert!(matches!(
            StateSource::default(),
            StateSource::LocalEncrypted
        ));
    }
}
