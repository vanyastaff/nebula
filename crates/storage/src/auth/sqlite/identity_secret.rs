//! Startup authentication and current-key rotation of persisted identity envelopes.
//! SQLite identity has no historical plaintext format to adopt.

use crate::{
    StorageError,
    auth::identity_secret::{IdentitySecretCodec, TotpSecretPurpose},
    sql_error::storage_error,
};
use sqlx::SqlitePool;

const BATCH_SIZE: i64 = 128;

/// Authenticate every stored active/pending factor before exposing authentication.
/// Explicitly configured old-key envelopes are resealed with the current key.
///
/// One SQLite writer transaction excludes concurrent factor changes. Queries are
/// memory-bounded; a failure rolls back all replacements. No schema setup or new
/// connection pool is created here.
///
/// # Errors
/// Fails closed on malformed, wrong-owner, wrong-purpose or unavailable-key
/// material and database failures. Diagnostics never include stored bytes.
#[tracing::instrument(level = "info", skip_all, fields(operation = "identity_secret_admit"))]
pub async fn admit_identity_secrets(
    pool: &SqlitePool,
    codec: &IdentitySecretCodec,
) -> Result<(), StorageError> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(storage_error)?;
    let mut examined = 0_u64;
    let mut rotated = 0_u64;
    let mut after: Option<Vec<u8>> = None;
    loop {
        let rows: Vec<(Vec<u8>, Option<Vec<u8>>)> = sqlx::query_as(
            "SELECT id, mfa_secret_envelope FROM users
             WHERE (mfa_enabled = 1 OR mfa_secret_envelope IS NOT NULL)
               AND (?1 IS NULL OR id > ?1) ORDER BY id LIMIT ?2",
        )
        .bind(after.as_deref())
        .bind(BATCH_SIZE)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        if rows.is_empty() {
            break;
        }
        for (id, envelope) in rows {
            let envelope = envelope.ok_or_else(|| {
                StorageError::Corrupt("active MFA account has no envelope".into())
            })?;
            let opened = codec
                .open_totp_seed(TotpSecretPurpose::Active, &id, &envelope)
                .map_err(|_| {
                    StorageError::Corrupt("active identity secret rejected at startup".into())
                })?;
            if let Some(replacement) = opened.replacement_envelope {
                sqlx::query(
                    "UPDATE users SET mfa_secret_envelope = ?, version = version + 1 WHERE id = ?",
                )
                .bind(replacement)
                .bind(&id)
                .execute(&mut *tx)
                .await
                .map_err(storage_error)?;
                rotated += 1;
            }
            examined += 1;
            after = Some(id);
        }
    }
    after = None;
    loop {
        let rows: Vec<(Vec<u8>, Vec<u8>)> = sqlx::query_as(
            "SELECT user_id, secret_envelope FROM mfa_enrollment_candidates
             WHERE (?1 IS NULL OR user_id > ?1) ORDER BY user_id LIMIT ?2",
        )
        .bind(after.as_deref())
        .bind(BATCH_SIZE)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage_error)?;
        if rows.is_empty() {
            break;
        }
        for (id, envelope) in rows {
            let opened = codec
                .open_totp_seed(TotpSecretPurpose::EnrollmentCandidate, &id, &envelope)
                .map_err(|_| {
                    StorageError::Corrupt("pending identity secret rejected at startup".into())
                })?;
            if let Some(replacement) = opened.replacement_envelope {
                sqlx::query(
                    "UPDATE mfa_enrollment_candidates SET secret_envelope = ? WHERE user_id = ?",
                )
                .bind(replacement)
                .bind(&id)
                .execute(&mut *tx)
                .await
                .map_err(storage_error)?;
                rotated += 1;
            }
            examined += 1;
            after = Some(id);
        }
    }
    tx.commit().await.map_err(storage_error)?;
    tracing::info!(examined, rotated, "identity secrets admitted");
    Ok(())
}
