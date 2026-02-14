//! Rotation Backup System
//!
//! Creates encrypted backups of credentials before rotation for disaster recovery.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};

use crate::core::CredentialId;
use crate::utils::EncryptedData;

use super::error::{RotationError, RotationResult};
use super::transaction::{BackupId, RotationId};

/// Rotation backup containing encrypted credential snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationBackup {
    /// Unique backup identifier
    pub id: BackupId,

    /// Credential being backed up
    pub credential_id: CredentialId,

    /// Credential version at time of backup
    pub credential_version: u32,

    /// Encrypted credential data
    pub encrypted_data: EncryptedData,

    /// Rotation transaction this backup belongs to
    pub transaction_id: RotationId,

    /// When backup was created
    pub created_at: DateTime<Utc>,

    /// When backup expires (minimum 30 days)
    pub expires_at: DateTime<Utc>,
}

impl RotationBackup {
    /// Create a new rotation backup
    pub fn create(
        credential_id: CredentialId,
        credential_version: u32,
        encrypted_data: EncryptedData,
        transaction_id: RotationId,
    ) -> Self {
        let created_at = Utc::now();
        let expires_at = created_at + ChronoDuration::days(30); // 30-day minimum retention

        Self {
            id: BackupId::new(),
            credential_id,
            credential_version,
            encrypted_data,
            transaction_id,
            created_at,
            expires_at,
        }
    }

    /// Restore credential from backup
    ///
    /// # Security Note
    ///
    /// This method clones the encrypted data, creating a temporary copy in memory.
    /// While the data is encrypted, this increases the attack surface for memory
    /// scraping. The original `EncryptedData` implements `Drop` to zero memory,
    /// but the clone will exist until the caller drops it.
    ///
    /// Consider using this sparingly and ensuring the returned data is dropped
    /// promptly after use.
    pub fn restore(&self) -> RotationResult<EncryptedData> {
        // Check if backup is expired
        if Utc::now() > self.expires_at {
            return Err(RotationError::RestoreFailed {
                backup_id: self.id.to_string(),
                reason: "Backup has expired".to_string(),
            });
        }

        tracing::debug!(
            backup_id = %self.id,
            "Cloning encrypted data for restore - temporary copy in memory"
        );

        Ok(self.encrypted_data.clone())
    }

    /// Check if backup has expired
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Extend backup expiration (for grace period extension)
    pub fn extend_expiration(&mut self, additional_days: i64) -> RotationResult<()> {
        self.expires_at += ChronoDuration::days(additional_days);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_creation() {
        let cred_id = CredentialId::new("test-cred").unwrap();
        let tx_id = RotationId::new();
        let encrypted = EncryptedData {
            version: 1,
            nonce: [4u8; 12],
            ciphertext: vec![1, 2, 3],
            tag: [5u8; 16],
        };

        let backup = RotationBackup::create(cred_id.clone(), 1, encrypted.clone(), tx_id);

        assert_eq!(backup.credential_id, cred_id);
        assert_eq!(backup.credential_version, 1);
        assert!(!backup.is_expired());
    }

    #[test]
    fn test_backup_restore() {
        let cred_id = CredentialId::new("test-cred").unwrap();
        let tx_id = RotationId::new();
        let encrypted = EncryptedData {
            version: 1,
            nonce: [4u8; 12],
            ciphertext: vec![1, 2, 3],
            tag: [5u8; 16],
        };

        let backup = RotationBackup::create(cred_id, 1, encrypted.clone(), tx_id);
        let restored = backup.restore().unwrap();

        assert_eq!(restored.ciphertext, encrypted.ciphertext);
        assert_eq!(restored.nonce, encrypted.nonce);
    }

    #[test]
    fn test_backup_expiration_extension() {
        let cred_id = CredentialId::new("test-cred").unwrap();
        let tx_id = RotationId::new();
        let encrypted = EncryptedData {
            version: 1,
            nonce: [4u8; 12],
            ciphertext: vec![1, 2, 3],
            tag: [5u8; 16],
        };

        let mut backup = RotationBackup::create(cred_id, 1, encrypted, tx_id);
        let original_expiry = backup.expires_at;

        backup.extend_expiration(10).unwrap();

        assert!(backup.expires_at > original_expiry);
        assert_eq!((backup.expires_at - original_expiry).num_days(), 10);
    }
}
