//! Bounded credential keyring configuration shared by first-party processes.

use std::{collections::HashSet, sync::Arc};

use nebula_crypto::EncryptionKey;
use zeroize::Zeroizing;

use super::{EnvKeyProvider, KeyProvider, ProviderError};

/// Comma-separated decrypt-only keys understood by current envelopes.
pub const LEGACY_MASTER_KEYS_ENV: &str = "NEBULA_CRED_LEGACY_MASTER_KEYS";
/// Credential-only key for historical envelopes whose key identifier is empty.
pub const LEGACY_EMPTY_ID_MASTER_KEY_ENV: &str = "NEBULA_CRED_LEGACY_EMPTY_ID_MASTER_KEY";
const MAX_LEGACY_MASTER_KEYS: usize = 8;

/// Validated current and decrypt-only keys for first-party credential storage.
pub struct CredentialKeyring {
    current: Arc<dyn KeyProvider>,
    legacy: Vec<(String, Arc<EncryptionKey>)>,
    credential_empty_id_key: Option<Arc<EncryptionKey>>,
}

impl CredentialKeyring {
    /// Read and validate the decrypt-only key configuration from the process environment.
    pub fn from_env(current: Arc<dyn KeyProvider>) -> Result<Self, CredentialKeyringError> {
        let configured = optional_secret_env(LEGACY_MASTER_KEYS_ENV)?;
        let empty_id_configured = optional_secret_env(LEGACY_EMPTY_ID_MASTER_KEY_ENV)?;
        Self::from_config(
            current,
            configured.as_deref().map(String::as_str),
            empty_id_configured.as_deref().map(String::as_str),
        )
    }

    /// Validate explicit base64 configuration without reading process-global state.
    pub fn from_config(
        current: Arc<dyn KeyProvider>,
        configured: Option<&str>,
        empty_id_configured: Option<&str>,
    ) -> Result<Self, CredentialKeyringError> {
        let current_id = current
            .current()
            .map_err(CredentialKeyringError::CurrentKeyUnavailable)?
            .key_id()
            .to_owned();
        let encoded_keys: Vec<_> = configured
            .filter(|value| !value.trim().is_empty())
            .map_or_else(Vec::new, |value| value.split(',').map(str::trim).collect());
        if encoded_keys.len() > MAX_LEGACY_MASTER_KEYS {
            return Err(CredentialKeyringError::TooManyLegacyKeys);
        }
        if encoded_keys.iter().any(|key| key.is_empty()) {
            return Err(CredentialKeyringError::MalformedLegacyKey);
        }

        let mut seen = HashSet::with_capacity(encoded_keys.len());
        let mut legacy = Vec::with_capacity(encoded_keys.len());
        for encoded_key in encoded_keys {
            let snapshot = parse_key(encoded_key)?;
            let (key_id, key) = snapshot.into_parts();
            let key_id = key_id.to_string();
            if key_id == current_id {
                return Err(CredentialKeyringError::CurrentKeyListedAsLegacy);
            }
            if !seen.insert(key_id.clone()) {
                return Err(CredentialKeyringError::DuplicateLegacyKey);
            }
            legacy.push((key_id, key));
        }
        let credential_empty_id_key = empty_id_configured
            .map(str::trim)
            .map(|encoded_key| {
                if encoded_key.is_empty() {
                    return Err(CredentialKeyringError::MalformedEmptyIdKey);
                }
                parse_key(encoded_key)
                    .map(|snapshot| snapshot.into_parts().1)
                    .map_err(|_| CredentialKeyringError::MalformedEmptyIdKey)
            })
            .transpose()?;
        Ok(Self {
            current,
            legacy,
            credential_empty_id_key,
        })
    }

    /// Current provider used for every new envelope.
    pub fn current(&self) -> Arc<dyn KeyProvider> {
        Arc::clone(&self.current)
    }

    /// Non-empty decrypt-only keys accepted by Plane-A identity storage.
    pub fn identity_legacy(&self) -> Vec<(String, Arc<EncryptionKey>)> {
        self.legacy.clone()
    }

    /// Decrypt-only keys accepted by credential storage, including the explicit empty alias.
    pub fn credential_legacy(&self) -> Vec<(String, Arc<EncryptionKey>)> {
        let mut legacy = self.legacy.clone();
        if let Some(key) = &self.credential_empty_id_key {
            legacy.push((String::new(), Arc::clone(key)));
        }
        legacy
    }
}

fn parse_key(encoded_key: &str) -> Result<super::KeySnapshot, CredentialKeyringError> {
    EnvKeyProvider::from_base64(encoded_key)
        .and_then(|provider| provider.current())
        .map_err(|_| CredentialKeyringError::MalformedLegacyKey)
}

fn optional_secret_env(
    name: &'static str,
) -> Result<Option<Zeroizing<String>>, CredentialKeyringError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(Zeroizing::new(value))),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(CredentialKeyringError::NonUnicodeEnvironment { name })
        },
    }
}

/// Secret-free reason a credential keyring was rejected.
#[derive(Debug, thiserror::Error)]
pub enum CredentialKeyringError {
    /// The current provider could not produce a key snapshot.
    #[error("current credential key is unavailable")]
    CurrentKeyUnavailable(#[source] ProviderError),
    /// A configured environment value is not valid Unicode.
    #[error("credential keyring environment value is not Unicode: {name}")]
    NonUnicodeEnvironment {
        /// Name of the rejected environment variable; never its value.
        name: &'static str,
    },
    /// More decrypt-only keys were configured than the bounded policy permits.
    #[error("credential keyring contains too many legacy keys")]
    TooManyLegacyKeys,
    /// A decrypt-only key is empty or cannot be decoded as AES-256 material.
    #[error("credential keyring contains a malformed legacy key")]
    MalformedLegacyKey,
    /// The current key was redundantly listed as a normal legacy generation.
    #[error("credential keyring lists the current key as legacy")]
    CurrentKeyListedAsLegacy,
    /// One decrypt-only generation was configured more than once.
    #[error("credential keyring contains a duplicate legacy key")]
    DuplicateLegacyKey,
    /// The historical empty-ID alias is present but malformed.
    #[error("credential empty-ID legacy key is malformed")]
    MalformedEmptyIdKey,
}

impl CredentialKeyringError {
    /// Stable secret-free category for startup telemetry.
    pub const fn category(&self) -> &'static str {
        match self {
            Self::CurrentKeyUnavailable(_) => "current_key_unavailable",
            Self::NonUnicodeEnvironment { .. } => "non_unicode_environment",
            Self::TooManyLegacyKeys => "too_many_legacy_keys",
            Self::MalformedLegacyKey => "malformed_legacy_key",
            Self::CurrentKeyListedAsLegacy => "current_key_listed_as_legacy",
            Self::DuplicateLegacyKey => "duplicate_legacy_key",
            Self::MalformedEmptyIdKey => "malformed_empty_id_key",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn malformed_empty_id_key_has_its_own_safe_category() {
        let encoded_current = base64::engine::general_purpose::STANDARD.encode([1_u8; 32]);
        let current: Arc<dyn KeyProvider> = Arc::new(
            EnvKeyProvider::from_base64(&encoded_current).expect("valid current test key"),
        );

        let error = match CredentialKeyring::from_config(current, None, Some("not-base64")) {
            Err(error) => error,
            Ok(_) => panic!("malformed empty-ID key must fail closed"),
        };

        assert!(matches!(error, CredentialKeyringError::MalformedEmptyIdKey));
        assert_eq!(error.category(), "malformed_empty_id_key");
    }
}
