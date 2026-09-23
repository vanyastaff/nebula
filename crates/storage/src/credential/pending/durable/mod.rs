//! Durable encrypted pending-state stores for interactive credential flows.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use nebula_credential::{DynPendingStateStore, PendingStoreError, PendingToken};
use nebula_crypto::{EncryptedData, EncryptionKey, decrypt_with_aad, encrypt_with_key_id};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::super::key_provider::{KeyProvider, KeySnapshot};

const AAD_DOMAIN: &[u8] = b"nebula:credential-pending:v1";
const INSERT_ATTEMPTS: usize = 4;

#[derive(Debug, thiserror::Error)]
enum DurablePendingError {
    #[error("pending state storage unavailable")]
    Unavailable,
    #[error("pending state record is corrupt")]
    CorruptRecord,
}

fn backend(error: DurablePendingError) -> PendingStoreError {
    PendingStoreError::Backend(Box::new(error))
}

fn token_digest(token: &PendingToken) -> [u8; 32] {
    Sha256::digest(token.as_str().as_bytes()).into()
}

fn append_aad_field(aad: &mut Vec<u8>, value: &[u8]) -> Result<(), DurablePendingError> {
    let len = u64::try_from(value.len()).map_err(|_| DurablePendingError::Unavailable)?;
    aad.extend_from_slice(&len.to_be_bytes());
    aad.extend_from_slice(value);
    Ok(())
}

fn pending_aad(
    digest: &[u8; 32],
    credential_kind: &str,
    owner_id: &str,
    session_id: &str,
    expires_at_ms: i64,
) -> Result<Vec<u8>, DurablePendingError> {
    let mut aad = Vec::with_capacity(
        AAD_DOMAIN.len()
            + digest.len()
            + credential_kind.len()
            + owner_id.len()
            + session_id.len()
            + 48,
    );
    aad.extend_from_slice(AAD_DOMAIN);
    append_aad_field(&mut aad, digest)?;
    append_aad_field(&mut aad, credential_kind.as_bytes())?;
    append_aad_field(&mut aad, owner_id.as_bytes())?;
    append_aad_field(&mut aad, session_id.as_bytes())?;
    aad.extend_from_slice(&expires_at_ms.to_be_bytes());
    Ok(aad)
}

struct PendingCipher {
    key_provider: Arc<dyn KeyProvider>,
    legacy_keys: HashMap<String, Arc<EncryptionKey>>,
}

impl PendingCipher {
    fn new(
        key_provider: Arc<dyn KeyProvider>,
        legacy_keys: Vec<(String, Arc<EncryptionKey>)>,
    ) -> Self {
        Self {
            key_provider,
            legacy_keys: legacy_keys.into_iter().collect(),
        }
    }

    fn current(&self) -> Result<KeySnapshot, DurablePendingError> {
        self.key_provider
            .current()
            .map_err(|_| DurablePendingError::Unavailable)
    }

    fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, DurablePendingError> {
        let current = self.current()?;
        let envelope = encrypt_with_key_id(current.key(), current.key_id(), plaintext, aad)
            .map_err(|_| DurablePendingError::Unavailable)?;
        serde_json::to_vec(&envelope).map_err(|_| DurablePendingError::Unavailable)
    }

    fn decrypt(&self, bytes: &[u8], aad: &[u8]) -> Result<Zeroizing<Vec<u8>>, DurablePendingError> {
        let envelope: EncryptedData =
            serde_json::from_slice(bytes).map_err(|_| DurablePendingError::CorruptRecord)?;
        let current = self.current()?;
        if envelope.key_id == current.key_id() {
            return decrypt_with_aad(current.key(), &envelope, aad)
                .map_err(|_| DurablePendingError::CorruptRecord);
        }
        let legacy = self
            .legacy_keys
            .get(&envelope.key_id)
            .ok_or(DurablePendingError::CorruptRecord)?;
        decrypt_with_aad(legacy, &envelope, aad).map_err(|_| DurablePendingError::CorruptRecord)
    }
}

impl std::fmt::Debug for PendingCipher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingCipher")
            .field("legacy_key_count", &self.legacy_keys.len())
            .finish_non_exhaustive()
    }
}

fn expiry_from(now_ms: i64, expires_in: Duration) -> Result<i64, PendingStoreError> {
    super::validate_pending_ttl(expires_in)?;
    let ttl_ms = i64::try_from(expires_in.as_millis())
        .map_err(|_| backend(DurablePendingError::Unavailable))?;
    now_ms
        .checked_add(ttl_ms)
        .ok_or_else(|| backend(DurablePendingError::Unavailable))
}

fn binding_matches(row: &PendingRow, kind: &str, owner: &str, session: &str) -> bool {
    row.credential_kind == kind && row.owner_id == owner && row.session_id == session
}

struct PendingRow {
    credential_kind: String,
    owner_id: String,
    session_id: String,
    state_encrypted: Vec<u8>,
    expires_at_ms: i64,
    expired: bool,
}

impl PendingRow {
    fn decrypt(
        &self,
        cipher: &PendingCipher,
        digest: &[u8; 32],
    ) -> Result<Zeroizing<Vec<u8>>, PendingStoreError> {
        let aad = pending_aad(
            digest,
            &self.credential_kind,
            &self.owner_id,
            &self.session_id,
            self.expires_at_ms,
        )
        .map_err(backend)?;
        cipher.decrypt(&self.state_encrypted, &aad).map_err(backend)
    }
}

#[cfg(feature = "sqlite")]
mod sqlite;
#[cfg(feature = "sqlite")]
pub use sqlite::SqlitePendingStateStore;

#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "postgres")]
pub use postgres::PgPendingStateStore;

#[cfg(test)]
mod migration_contract_tests {
    #[test]
    fn dialects_define_the_same_pending_state_contract() {
        let sqlite =
            include_str!("../../../../migrations/sqlite/0055_credential_pending_states.sql");
        let postgres =
            include_str!("../../../../migrations/postgres/0055_credential_pending_states.sql");
        for column in [
            "token_digest",
            "credential_kind",
            "owner_id",
            "session_id",
            "state_encrypted",
            "created_at",
            "expires_at",
        ] {
            assert!(sqlite.contains(column), "SQLite migration misses {column}");
            assert!(
                postgres.contains(column),
                "Postgres migration misses {column}"
            );
        }
        assert!(sqlite.contains("length(token_digest) = 32"));
        assert!(postgres.contains("octet_length(token_digest) = 32"));
        assert!(sqlite.contains("CHECK (expires_at >= created_at)"));
        assert!(postgres.contains("CHECK (expires_at >= created_at)"));
    }

    #[test]
    fn backend_sources_use_backend_clocks_and_lock_before_consume() {
        let sqlite_source = include_str!("sqlite.rs");
        let postgres_source = include_str!("postgres.rs");
        assert!(sqlite_source.contains("begin_with(\"BEGIN IMMEDIATE\")"));
        assert!(postgres_source.contains("expires_at <= clock_timestamp()"));
        assert!(postgres_source.contains("FOR UPDATE"));
    }
}
