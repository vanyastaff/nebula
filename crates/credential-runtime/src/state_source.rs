//! Polymorphic credential state source. Replaces the resolver's
//! hardcoded "state always from `CredentialStore`" (spec §8 — no
//! adapter/bridge). `External` fulfils ADR-0051's deferred Phase-D
//! non-goal: resolved secrets carrying a lease are tracked via
//! `LeaseLifecycle`.

use std::sync::Arc;

use nebula_credential::provider::ExternalProvider;

/// Where a credential's resolved material comes from.
#[derive(Default)]
pub enum StateSource {
    /// The crate-private layered encrypted store (default).
    #[default]
    LocalEncrypted,
    /// An external secret provider chain (Vault, etc.). A
    /// `ProviderResolution` carrying a lease is handed to
    /// `LeaseLifecycle::track`.
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
