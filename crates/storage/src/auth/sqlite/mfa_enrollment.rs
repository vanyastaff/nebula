//! Pending MFA candidates stay separate from the active factor until installation.

use super::{NOW, instant};
use crate::{
    StorageError,
    auth::{
        MfaEnrollmentCandidate, MfaEnrollmentInstallOutcome, MfaEnrollmentRepo,
        identity_secret::{IdentitySecretCodec, IdentitySecretError, TotpSecretPurpose},
    },
    sql_error::{storage_error, storage_error_for},
};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use std::sync::Arc;

/// SQLite owner of exact-candidate MFA installation.
#[derive(Clone)]
pub struct SqliteMfaEnrollmentRepo {
    pool: SqlitePool,
    identity_secrets: Arc<IdentitySecretCodec>,
}

impl SqliteMfaEnrollmentRepo {
    /// Bind to the deployment pool and its shared identity codec.
    #[must_use]
    pub fn new(pool: SqlitePool, identity_secrets: Arc<IdentitySecretCodec>) -> Self {
        Self {
            pool,
            identity_secrets,
        }
    }
}

fn decode_candidate(row: SqliteRow) -> Result<MfaEnrollmentCandidate, StorageError> {
    let id: Vec<u8> = row.try_get("enrollment_id").map_err(storage_error)?;
    MfaEnrollmentCandidate::new(
        id.try_into()
            .map_err(|_| StorageError::Corrupt("MFA enrollment id is not 32 bytes".into()))?,
        row.try_get("user_id").map_err(storage_error)?,
        row.try_get("secret_envelope").map_err(storage_error)?,
        instant(&row, "created_at")?,
        instant(&row, "expires_at")?,
    )
    .map_err(|_| StorageError::Corrupt("invalid stored MFA candidate".into()))
}

fn secret_error(_: IdentitySecretError) -> StorageError {
    StorageError::Corrupt("identity secret envelope operation failed".into())
}

#[async_trait::async_trait]
impl MfaEnrollmentRepo for SqliteMfaEnrollmentRepo {
    #[tracing::instrument(level = "debug", skip_all)]
    async fn replace_candidate(
        &self,
        candidate: &MfaEnrollmentCandidate,
    ) -> Result<(), StorageError> {
        let opened = self
            .identity_secrets
            .open_totp_seed(
                TotpSecretPurpose::EnrollmentCandidate,
                candidate.user_id(),
                candidate.secret_envelope(),
            )
            .map_err(secret_error)?;
        let envelope = opened
            .replacement_envelope
            .as_deref()
            .unwrap_or_else(|| candidate.secret_envelope());
        let inserted = sqlx::query(
            "INSERT INTO mfa_enrollment_candidates
             (user_id, enrollment_id, secret_envelope, created_at, expires_at)
            SELECT id, ?2, ?3, ?4, ?5 FROM users WHERE id = ?1 AND deleted_at IS NULL
            ON CONFLICT (user_id) DO UPDATE SET
            enrollment_id = excluded.enrollment_id, secret_envelope = excluded.secret_envelope,
            created_at = excluded.created_at, expires_at = excluded.expires_at",
        )
        .bind(candidate.user_id())
        .bind(candidate.enrollment_id().as_slice())
        .bind(envelope)
        .bind(candidate.created_at().timestamp_micros())
        .bind(candidate.expires_at().timestamp_micros())
        .execute(&self.pool)
        .await
        .map_err(|error| storage_error_for("mfa_enrollment_candidate", error))?
        .rows_affected();
        if inserted == 0 {
            return Err(StorageError::not_found("user", "MFA enrollment owner"));
        }
        Ok(())
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn get_live_candidate(
        &self,
        user_id: &[u8],
    ) -> Result<Option<MfaEnrollmentCandidate>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT user_id, enrollment_id, secret_envelope, created_at, expires_at
            FROM mfa_enrollment_candidates WHERE user_id = ? AND expires_at > {NOW}
              AND EXISTS (SELECT 1 FROM users WHERE id = mfa_enrollment_candidates.user_id AND deleted_at IS NULL)"
        )))
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?
        .map(decode_candidate)
        .transpose()
    }

    #[tracing::instrument(level = "debug", skip_all)]
    async fn install_candidate(
        &self,
        user_id: &[u8],
        enrollment_id: &[u8; 32],
    ) -> Result<MfaEnrollmentInstallOutcome, StorageError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let owner: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT id FROM users WHERE id = ? AND deleted_at IS NULL")
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(storage_error)?;
        if owner.is_none() {
            return Err(StorageError::not_found("user", "MFA enrollment owner"));
        }
        let envelope: Option<Vec<u8>> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("DELETE FROM mfa_enrollment_candidates
            WHERE user_id = ? AND enrollment_id = ? AND expires_at > {NOW} RETURNING secret_envelope")))
            .bind(user_id).bind(enrollment_id.as_slice()).fetch_optional(&mut *tx).await.map_err(storage_error)?;
        let Some(envelope) = envelope else {
            tx.rollback().await.map_err(storage_error)?;
            return Ok(MfaEnrollmentInstallOutcome::CandidateUnavailable);
        };
        let opened = self
            .identity_secrets
            .open_totp_seed(TotpSecretPurpose::EnrollmentCandidate, user_id, &envelope)
            .map_err(secret_error)?;
        let active = self
            .identity_secrets
            .seal_totp_seed(TotpSecretPurpose::Active, user_id, &opened.plaintext)
            .map_err(secret_error)?;
        let updated = sqlx::query(
            "UPDATE users SET mfa_secret_envelope = ?, mfa_enabled = 1, version = version + 1
            WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(active)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?
        .rows_affected();
        if updated != 1 {
            tx.rollback().await.map_err(storage_error)?;
            return Err(StorageError::not_found("user", "MFA enrollment owner"));
        }
        tx.commit().await.map_err(storage_error)?;
        Ok(MfaEnrollmentInstallOutcome::Installed)
    }
}
