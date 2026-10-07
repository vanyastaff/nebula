//! SQLite OAuth finalization, serialized before inspecting any identity row.

use super::user::{COLUMNS, decode_user};
use crate::{
    StorageError,
    auth::{
        OAuthLoginFinalizeCommand, OAuthLoginFinalizeOutcome, OAuthLoginFinalized,
        OAuthLoginFinalizer, UserRow, oauth_login::validate_common_command,
        session_token::session_token_digest,
    },
    sql_error::storage_error,
};
use sqlx::{Sqlite, SqlitePool, Transaction};

/// SQLite owner of atomic user/link/session or MFA-challenge finalization.
#[derive(Clone)]
pub struct SqliteOAuthLoginFinalizer {
    pool: SqlitePool,
}

impl SqliteOAuthLoginFinalizer {
    /// Bind to the application's admitted pool.
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

enum Decision {
    Session(Box<UserRow>),
    Mfa,
    Reject(OAuthLoginFinalizeOutcome),
}

#[async_trait::async_trait]
impl OAuthLoginFinalizer for SqliteOAuthLoginFinalizer {
    #[tracing::instrument(level = "info", skip_all, fields(operation = "oauth_login_finalize"))]
    async fn finalize(
        &self,
        command: OAuthLoginFinalizeCommand,
    ) -> Result<OAuthLoginFinalizeOutcome, StorageError> {
        validate_common_command(&command)?;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        match finalize(&mut tx, &command).await {
            Ok(Decision::Session(user)) => {
                tx.commit().await.map_err(storage_error)?;
                tracing::info!(outcome = "finalized", "OAuth login finalized");
                Ok(OAuthLoginFinalizeOutcome::Finalized(Box::new(
                    OAuthLoginFinalized {
                        user: *user,
                        session_token: command.session.token,
                        session_expires_at: command.session.expires_at,
                    },
                )))
            },
            Ok(Decision::Mfa) => {
                tx.commit().await.map_err(storage_error)?;
                tracing::info!(outcome = "mfa_required", "OAuth login requires MFA");
                Ok(OAuthLoginFinalizeOutcome::MfaRequired)
            },
            Ok(Decision::Reject(outcome)) => {
                tx.rollback().await.map_err(storage_error)?;
                tracing::warn!(outcome = ?outcome, "OAuth login rejected");
                Ok(outcome)
            },
            Err(error) => {
                tx.rollback().await.map_err(storage_error)?;
                tracing::error!(outcome = "storage_error", "OAuth login finalization failed");
                Err(error)
            },
        }
    }
}

async fn finalize(
    tx: &mut Transaction<'_, Sqlite>,
    command: &OAuthLoginFinalizeCommand,
) -> Result<Decision, StorageError> {
    // BEGIN IMMEDIATE serializes competing subjects and account deletion. A
    // second finalizer reads the first one's committed link; no candidate user
    // needs to be created and later removed to resolve a uniqueness race.
    let linked: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT user_id FROM external_identities WHERE provider = ? AND subject = ?",
    )
    .bind(&command.provider)
    .bind(&command.subject)
    .fetch_optional(&mut **tx)
    .await
    .map_err(storage_error)?;
    let user = if let Some(id) = linked {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM users WHERE id = ? AND deleted_at IS NULL"
        )))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(storage_error)?;
        let Some(row) = row else {
            return Ok(Decision::Reject(
                OAuthLoginFinalizeOutcome::LinkedUserUnavailable,
            ));
        };
        decode_user(row)?
    } else {
        let Some(email) = command.verified_email.as_deref() else {
            return Ok(Decision::Reject(
                OAuthLoginFinalizeOutcome::VerifiedEmailRequired,
            ));
        };
        let row = sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO users
            (id, email, email_verified_at, display_name, avatar_url, created_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (lower(email)) WHERE deleted_at IS NULL DO NOTHING
             RETURNING {COLUMNS}"
        )))
        .bind(&command.candidate_user.id)
        .bind(email)
        .bind(command.candidate_user.created_at.timestamp_micros())
        .bind(&command.candidate_user.display_name)
        .bind(&command.candidate_user.avatar_url)
        .bind(command.candidate_user.created_at.timestamp_micros())
        .fetch_optional(&mut **tx)
        .await
        .map_err(storage_error)?;
        let Some(row) = row else {
            return Ok(Decision::Reject(
                OAuthLoginFinalizeOutcome::AccountLinkRequired,
            ));
        };
        let user = decode_user(row)?;
        sqlx::query(
            "INSERT INTO external_identities (provider, subject, user_id, email, linked_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&command.provider)
        .bind(&command.subject)
        .bind(&user.id)
        .bind(email)
        .bind(command.candidate_user.created_at.timestamp_micros())
        .execute(&mut **tx)
        .await
        .map_err(storage_error)?;
        user
    };
    if user.mfa_enabled {
        if user
            .mfa_secret_envelope
            .as_deref()
            .is_none_or(<[u8]>::is_empty)
        {
            return Err(StorageError::Corrupt(
                "active MFA account has no envelope".into(),
            ));
        }
        sqlx::query(
            "INSERT INTO verification_tokens (token_hash, user_id, kind, created_at, expires_at)
            VALUES (?, ?, 'mfa_challenge', ?, ?)",
        )
        .bind(command.mfa_challenge.token_hash.as_slice())
        .bind(&user.id)
        .bind(command.mfa_challenge.created_at.timestamp_micros())
        .bind(command.mfa_challenge.expires_at.timestamp_micros())
        .execute(&mut **tx)
        .await
        .map_err(storage_error)?;
        return Ok(Decision::Mfa);
    }
    let session = &command.session;
    let address = session
        .ip_address
        .as_deref()
        .map(|value| {
            value
                .parse::<std::net::IpAddr>()
                .map(|address| address.to_string())
                .map_err(|_| StorageError::InvalidInput("invalid session IP address".into()))
        })
        .transpose()?;
    sqlx::query(
        "INSERT INTO sessions
         (token_digest, user_id, created_at, last_active_at, expires_at, ip_address, user_agent)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(session_token_digest(&session.token).as_bytes().as_slice())
    .bind(&user.id)
    .bind(session.created_at.timestamp_micros())
    .bind(session.last_active_at.timestamp_micros())
    .bind(session.expires_at.timestamp_micros())
    .bind(address)
    .bind(&session.user_agent)
    .execute(&mut **tx)
    .await
    .map_err(storage_error)?;
    Ok(Decision::Session(Box::new(user)))
}
